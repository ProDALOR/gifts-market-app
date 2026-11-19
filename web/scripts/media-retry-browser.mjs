// Actual HTTP caching, real native image decoding and React visibility changes.
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { gzipSync } from "node:zlib";
import { chromium } from "playwright";

const requests = new Map(),
  ready = new Set();
const count = (name) => requests.get(name) || 0;
const svg =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M100 100L400 100L400 400z" fill="#fff"/></svg>';
let png;
const tgs = gzipSync(
  JSON.stringify({
    v: "5.7.4",
    w: 512,
    h: 512,
    fr: 30,
    ip: 0,
    op: 60,
    layers: [],
    assets: [],
  }),
);
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body><div id="fixture" style="padding:25px;height:2400px"></div><script type="module">
import React from 'react';import{createRoot}from'react-dom/client';import{GiftArtwork}from'/src/previews/gift-artwork.tsx';import{TraitPreview}from'/src/previews/trait-preview.tsx';
const root=createRoot(document.getElementById('fixture'));
window.mount=(name,fallback='/media/still.png')=>root.render(React.createElement(GiftArtwork,{media:{thumbnail_url:fallback,preview_url:'/media/'+name+'.svg'},size:'large',playback:'static',fallback:'G'}));
window.clear=()=>root.render(null);
window.motion=(name='motion')=>root.render(React.createElement(GiftArtwork,{media:{thumbnail_url:'/media/still.png',animation_url:'/media/'+name+'.tgs'},size:'large',playback:'detail',fallback:'G'}));
window.manyMotions=()=>root.render(React.createElement('div',{style:{display:'flex'}},...Array.from({length:7},(_,i)=>React.createElement('div',{'data-model':i,key:i},React.createElement(GiftArtwork,{media:{thumbnail_url:'/media/still.png',animation_url:'/media/'+(i===6?'healthy':'blocked-'+i)+'.tgs'},playback:'selected',fallback:'G'})))));
window.symbols=names=>{const patterns=names.map(name=>({kind:'pattern',id:name,name,media:{thumbnail_url:'/media/'+name+'.png'}}));root.render(React.createElement(TraitPreview,{collection:{id:'gift',name:'Gift'},options:new Map(patterns.map(row=>[row.id,row])),filters:{patterns:names},reset:()=>{}}));};
window.visibility=visible=>{Object.defineProperty(document,'hidden',{configurable:true,get:()=>!visible});document.dispatchEvent(new Event('visibilitychange'));};
window.paintEvents=[];window.symbolDecodes=0;
const bitmap=createImageBitmap;window.createImageBitmap=(...args)=>{window.symbolDecodes++;return bitmap(...args);};
const raf=requestAnimationFrame;window.requestAnimationFrame=cb=>raf(time=>{window.frameTime=time;cb(time);});
const scale=CanvasRenderingContext2D.prototype.scale;CanvasRenderingContext2D.prototype.scale=function(...args){if(this.canvas.matches('.gift-backdrop')){this.canvas.hasSymbols=false;window.paintEvents.push(window.frameTime);}return scale.apply(this,args);};
const draw=CanvasRenderingContext2D.prototype.drawImage;CanvasRenderingContext2D.prototype.drawImage=function(...args){if(this.canvas.matches('.gift-backdrop')){this.canvas.hasSymbols=true;}return draw.apply(this,args);};
// Advance long cooldowns without faking animation frames or browser image decoding.
const now=Date.now,timeout=setTimeout,clear=clearTimeout;let offset=0,next=-1;const timers=new Map();Date.now=()=>now()+offset;
window.setTimeout=(fn,ms,...args)=>{if(ms>=20000){const id=next--;timers.set(id,{at:Date.now()+ms,run:()=>fn(...args)});return id;}return timeout(fn,ms,...args);};
window.clearTimeout=id=>{timers.delete(id);clear(id);};
window.advance=ms=>{offset+=ms;for(const[id,timer]of timers){if(timer.at<=Date.now()){timers.delete(id);timer.run();}}};
window.fixtureReady=true;
</script></body></html>`;
const server = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "media-retry-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url === "/__retry_test") {
            res.setHeader("Content-Type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
            return;
          }
          if (!req.url.startsWith("/media/")) return next();
          requests.set(req.url, count(req.url) + 1);
          res.setHeader("Vary", "Origin, Accept-Encoding");
          res.setHeader("Cache-Control", "public,max-age=86400");
          res.setHeader(
            "Content-Type",
            req.url.endsWith(".tgs")
              ? "application/x-tgsticker"
              : req.url.endsWith(".svg")
                ? "image/svg+xml"
                : "image/png",
          );
          if (req.url.startsWith("/media/missing")) {
            res.statusCode = 404;
            res.setHeader("Cache-Control", "no-store");
            res.end();
          } else if (
            (req.url === "/media/recover.png" ||
              req.url === "/media/cancel.png" ||
              req.url.endsWith(".svg") ||
              req.url.endsWith(".tgs") ||
              req.url.startsWith("/media/symbol-")) &&
            req.url !== "/media/invalid.svg" &&
            !ready.has(req.url)
          ) {
            res.statusCode = 503;
            res.setHeader("Cache-Control", "no-store");
            res.setHeader("Retry-After", "60");
            res.end();
          } else
            res.end(
              req.url === "/media/invalid.svg"
                ? "invalid SVG"
                : req.url.endsWith(".svg")
                  ? svg
                  : req.url.endsWith(".tgs")
                    ? tgs
                    : png,
            );
        });
      },
    },
  ],
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 800, height: 600 } });
  png = Buffer.from(
    await page.evaluate(() => {
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 16;
      const context = canvas.getContext("2d");
      context.fillStyle = "white";
      context.fillRect(3, 2, 10, 12);
      return canvas.toDataURL("image/png").split(",")[1];
    }),
    "base64",
  );
  page.setDefaultTimeout(10000);
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__retry_test`,
  );
  await page.waitForFunction(() => window.fixtureReady);
  async function pending(name) {
    await page.evaluate((name) => window.mount(name), name);
    await page.locator('img.loaded[src="/media/still.png"]').waitFor();
    for (let i = 0; i < 100 && count(`/media/${name}.svg`) < 2; i++)
      await page.waitForTimeout(10);
    assert.equal(
      count(`/media/${name}.svg`),
      2,
      "Original image plus one shared recovery attempt",
    );
    await page.waitForTimeout(40);
  }
  await pending("recover");
  ready.add("/media/recover.svg");
  await page.evaluate(() => window.advance(59_000));
  await page.waitForTimeout(30);
  assert.equal(count("/media/recover.svg"), 2);
  await page.evaluate(() => window.advance(1100));
  await page.locator('img.loaded[src="/media/recover.svg"]').waitFor();
  assert.equal(
    count("/media/recover.svg"),
    3,
    "Decode and remount reuse the warmed HTTP cache",
  );

  // A terminal preferred-image miss must not starve a transient fallback.
  await page.evaluate(() => window.mount("missing", "/media/recover.png"));
  await page.waitForFunction(() => !document.querySelector("img"));
  for (let i = 0; i < 100 && count("/media/recover.png") < 2; i++)
    await page.waitForTimeout(10);
  assert.equal(count("/media/recover.png"), 2);
  assert.equal(count("/media/missing.svg"), 2);
  ready.add("/media/recover.png");
  await page.evaluate(() => window.advance(60_100));
  await page.locator('img.loaded[src="/media/recover.png"]').waitFor();
  assert.equal(count("/media/recover.png"), 3);
  assert.equal(
    count("/media/missing.svg"),
    2,
    "Fallback recovery does not poll a terminal SVG miss",
  );
  assert.equal(await page.locator(".gift-loading-placeholder").count(), 0);

  await page.evaluate(() => window.mount("cancel-both", "/media/cancel.png"));
  for (let i = 0; i < 100 && count("/media/cancel.png") < 2; i++)
    await page.waitForTimeout(10);
  assert.equal(count("/media/cancel.png"), 2);
  assert.equal(count("/media/cancel-both.svg"), 2);
  await page.evaluate(() => window.clear());
  await page.waitForFunction(() => !document.querySelector(".gift-preview"));
  ready.add("/media/cancel.png");
  ready.add("/media/cancel-both.svg");
  await page.evaluate(() => window.advance(60_100));
  await page.waitForTimeout(30);
  assert.equal(
    count("/media/cancel.png"),
    2,
    "Unmount cancels fallback retries",
  );
  assert.equal(
    count("/media/cancel-both.svg"),
    2,
    "Unmount cancels preferred retries",
  );

  await pending("stale");
  await page.evaluate(() => window.mount("recover"));
  await page.locator('img.loaded[src="/media/recover.svg"]').waitFor();
  ready.add("/media/stale.svg");
  await page.evaluate(() => window.advance(60_100));
  await page.locator('img.loaded[src="/media/recover.svg"]').waitFor();
  assert.equal(
    count("/media/stale.svg"),
    2,
    "Changing artwork cancels retries owned by the previous source",
  );
  assert.equal(await page.locator('img[src="/media/stale.svg"]').count(), 0);

  await pending("reenter");
  await page.evaluate(() => window.scrollTo(0, 1300));
  await page.waitForTimeout(100);
  await page.evaluate(() => window.advance(30_000));
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.waitForTimeout(100);
  assert.equal(
    count("/media/reenter.svg"),
    2,
    "Reentry cannot bypass a shared cooldown",
  );
  ready.add("/media/reenter.svg");
  await page.evaluate(() => window.advance(30_100));
  await page.locator('img.loaded[src="/media/reenter.svg"]').waitFor();
  assert.equal(count("/media/reenter.svg"), 3);

  await pending("cancel");
  await page.evaluate(() => window.clear());
  await page.waitForFunction(() => !document.querySelector(".gift-preview"));
  ready.add("/media/cancel.svg");
  await page.evaluate(() => window.advance(60_100));
  await page.waitForTimeout(30);
  assert.equal(
    count("/media/cancel.svg"),
    2,
    "Unmount cancels scheduled retries",
  );
  await page.evaluate(() => window.mount("cancel"));
  await page.locator('img.loaded[src="/media/cancel.svg"]').waitFor();

  await page.evaluate(() => window.mount("invalid"));
  await page.locator('img.loaded[src="/media/still.png"]').waitFor();
  await page.waitForTimeout(250);
  const invalidRequests = count("/media/invalid.svg");
  await page.evaluate(() => window.scrollTo(0, 1300));
  await page.waitForTimeout(80);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.waitForTimeout(250);
  assert.equal(
    count("/media/invalid.svg"),
    invalidRequests,
    "Invalid 200 responses cannot cause a decode/remount loop",
  );
  assert.equal(await page.locator('img[src="/media/invalid.svg"]').count(), 0);
  // Collection switches and virtualized rows remount native images.
  await pending("remount");
  await page.evaluate(() => window.clear());
  await page.waitForFunction(() => !document.querySelector(".gift-preview"));
  await page.evaluate(() => window.mount("remount"));
  await page.locator('img.loaded[src="/media/still.png"]').waitFor();
  await page.waitForTimeout(80);
  assert.equal(
    count("/media/remount.svg"),
    2,
    "Remount cannot bypass Retry-After through a native image",
  );
  ready.add("/media/remount.svg");
  await page.evaluate(() => window.advance(59_000));
  await page.waitForTimeout(30);
  assert.equal(count("/media/remount.svg"), 2);
  await page.evaluate(() => window.advance(1100));
  await page.locator('img.loaded[src="/media/remount.svg"]').waitFor();
  assert.equal(count("/media/remount.svg"), 3);

  await pending("missing-remount");
  await page.evaluate(() => window.clear());
  await page.waitForFunction(() => !document.querySelector(".gift-preview"));
  await page.evaluate(() => window.mount("missing-remount"));
  await page.locator('img.loaded[src="/media/still.png"]').waitFor();
  await page.waitForTimeout(80);
  assert.equal(
    count("/media/missing-remount.svg"),
    2,
    "Terminal misses share cooldowns across mounts too",
  );

  async function requested(url, expected = 1) {
    for (let i = 0; i < 200 && count(url) < expected; i++)
      await page.waitForTimeout(10);
    assert.equal(count(url), expected, url);
    await page.waitForTimeout(80);
  }
  async function motion(name) {
    await page.evaluate((name) => window.motion(name), name);
    await requested("/media/" + name + ".tgs");
  }
  await motion("motion");
  ready.add("/media/motion.tgs");
  await page.evaluate(() => window.advance(59_000));
  await page.waitForTimeout(30);
  assert.equal(count("/media/motion.tgs"), 1);
  await page.evaluate(() => window.advance(1100));
  await page.locator(".model-animation.ready > svg").waitFor();
  assert.equal(
    count("/media/motion.tgs"),
    2,
    "Visible animation recovers without a new hover",
  );

  await motion("motion-hidden");
  await page.evaluate(() => window.visibility(false));
  await page.waitForTimeout(80);
  ready.add("/media/motion-hidden.tgs");
  await page.evaluate(() => window.advance(60_100));
  await page.waitForTimeout(80);
  assert.equal(
    count("/media/motion-hidden.tgs"),
    1,
    "Hidden tabs cancel animation retry waits",
  );
  await page.evaluate(() => window.visibility(true));
  await page.locator(".model-animation.ready > svg").waitFor();
  assert.equal(count("/media/motion-hidden.tgs"), 2);

  await motion("motion-scroll");
  await page.evaluate(() => window.scrollTo(0, 1300));
  await page.waitForTimeout(100);
  ready.add("/media/motion-scroll.tgs");
  await page.evaluate(() => window.advance(60_100));
  await page.waitForTimeout(80);
  assert.equal(
    count("/media/motion-scroll.tgs"),
    1,
    "Scrolling away cancels animation retries",
  );
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.locator(".model-animation.ready > svg").waitFor();
  assert.equal(count("/media/motion-scroll.tgs"), 2);

  await motion("motion-stale");
  await page.evaluate(() => window.motion("motion"));
  await page.locator(".model-animation.ready > svg").waitFor();
  ready.add("/media/motion-stale.tgs");
  await page.evaluate(() => window.advance(60_100));
  await page.waitForTimeout(80);
  assert.equal(
    count("/media/motion-stale.tgs"),
    1,
    "Switching artwork cancels animation retries",
  );

  ready.add("/media/healthy.tgs");
  await page.evaluate(() => window.manyMotions());
  await page
    .locator('[data-model="6"] .model-animation.ready canvas')
    .waitFor();
  assert.ok((await page.locator(".model-animation.ready").count()) <= 6);
  for (let i = 0; i < 6; i++)
    assert.equal(count("/media/blocked-" + i + ".tgs"), 1);
  await page.evaluate(() => window.clear());
  await page.waitForFunction(() => !document.querySelector(".gift-preview"));
  for (let i = 0; i < 6; i++) ready.add("/media/blocked-" + i + ".tgs");
  await page.evaluate(() => window.advance(60_100));
  await page.waitForTimeout(80);
  for (let i = 0; i < 6; i++)
    assert.equal(
      count("/media/blocked-" + i + ".tgs"),
      1,
      "Cooldowns release playback slots and unmount cancels recovery",
    );

  await page.evaluate(() => window.symbols(["symbol-recover"]));
  await requested("/media/symbol-recover.png");
  ready.add("/media/symbol-recover.png");
  await page.evaluate(() => window.advance(60_100));
  await page.waitForFunction(
    () => document.querySelector(".gift-backdrop")?.hasSymbols,
  );
  assert.equal(
    count("/media/symbol-recover.png"),
    2,
    "Visible symbols recover without scrolling",
  );

  await page.evaluate(() => window.symbols(["symbol-hidden"]));
  await requested("/media/symbol-hidden.png");
  await page.evaluate(() => window.visibility(false));
  await page.waitForFunction(
    () => document.querySelector(".gift-backdrop")?.width === 1,
  );
  ready.add("/media/symbol-hidden.png");
  await page.evaluate(() => window.advance(60_100));
  await page.waitForTimeout(80);
  assert.equal(
    count("/media/symbol-hidden.png"),
    1,
    "Hidden selected wallpaper cancels retry waits",
  );
  await page.evaluate(() => window.visibility(true));
  await requested("/media/symbol-hidden.png", 2);
  await page.waitForFunction(
    () =>
      document.querySelector(".gift-backdrop")?.width > 1 &&
      document.querySelector(".gift-backdrop")?.hasSymbols,
  );

  await page.evaluate(() => window.symbols(["symbol-stale"]));
  await requested("/media/symbol-stale.png");
  await page.evaluate(() => window.symbols(["symbol-recover"]));
  await page.waitForTimeout(80);
  ready.add("/media/symbol-stale.png");
  await page.evaluate(() => window.advance(60_100));
  await page.waitForTimeout(80);
  assert.equal(
    count("/media/symbol-stale.png"),
    1,
    "Changing selections cancels old symbol retries",
  );

  const blockedSymbols = Array.from(
    { length: 4 },
    (_, i) => "symbol-blocked-" + i,
  );
  ready.add("/media/symbol-healthy.png");
  await page.evaluate(
    (names) => window.symbols([...names, "symbol-healthy"]),
    blockedSymbols,
  );
  await requested("/media/symbol-healthy.png");
  await page.waitForFunction(
    () => document.querySelector(".gift-backdrop")?.hasSymbols,
  );
  for (const name of blockedSymbols)
    assert.equal(count("/media/" + name + ".png"), 1);
  await page.evaluate(() => window.clear());
  await page.waitForFunction(() => !document.querySelector(".gift-backdrop"));
  for (const name of blockedSymbols) ready.add("/media/" + name + ".png");
  await page.evaluate(() => window.advance(60_100));
  await page.waitForTimeout(80);
  for (const name of blockedSymbols)
    assert.equal(
      count("/media/" + name + ".png"),
      1,
      "Unmount cancels symbol recovery without blocking healthy symbols",
    );

  const batch = Array.from({ length: 20 }, (_, i) => "symbol-batch-" + i);
  for (const name of batch) ready.add("/media/" + name + ".png");
  await page.evaluate((batch) => window.symbols(batch), batch);
  for (const name of batch) await requested("/media/" + name + ".png");
  await page.waitForFunction(
    () => document.querySelector(".gift-backdrop")?.hasSymbols,
  );
  await page.evaluate(() => window.clear());
  await page.waitForFunction(() => !document.querySelector(".gift-backdrop"));
  const decodes = await page.evaluate(() => window.symbolDecodes);
  await page.evaluate((batch) => {
    window.paintEvents = [];
    window.symbols(batch);
  }, batch);
  await page.waitForFunction(
    () => document.querySelector(".gift-backdrop")?.hasSymbols,
  );
  const paints = await page.evaluate(() => window.paintEvents);
  assert.equal(
    await page.evaluate(() => window.symbolDecodes),
    decodes,
    "Remount reuses decoded symbols, not just the browser HTTP cache",
  );
  assert.equal(
    new Set(paints).size,
    paints.length,
    "Cached symbol completions and resizes paint at most once per frame",
  );
  for (const name of batch)
    assert.equal(
      count("/media/" + name + ".png"),
      1,
      "Decoded symbols are reused across remounts",
    );
  assert.deepEqual(errors, []);
  console.log(
    "Media retry browser passed: long Retry-After, visible recovery, cached decode/remount, cooldown reentry, remount cooldowns, automatic animation/symbol recovery, hidden-tab cleanup, playback admission, batched symbols and decoded-cache reuse.",
  );
} finally {
  await browser?.close();
  await server.close();
}
