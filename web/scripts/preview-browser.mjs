// Isolated browser regression: real React/Lottie, synthetic public media, no hub or Telegram.
import assert from "node:assert/strict";
import { gzipSync } from "node:zlib";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const root = fileURLToPath(new URL("..", import.meta.url));
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body>
<div id="fixture" style="margin:50px;height:4000px"></div>
<script type="module">
import React from 'react';
import {createRoot} from 'react-dom/client';
import {GiftPreview} from '/src/gift-preview.tsx';
import {CollectionRail,TraitPreview,VenueIcon} from '/src/orderbook-ui.tsx';
const order={attributes:Object.fromEntries(['model','backdrop','pattern'].map(id=>['tg.'+id,{op:'eq',value:{type:'enum',value:id}}]))};
const options=new Map([
 ['model',{kind:'attribute_option',id:'model',name:'Model',attribute_id:'tg.model',meta:{},aliases:[]}],
 ['backdrop',{id:'backdrop',name:'Backdrop',attribute_id:'tg.backdrop',meta:{'colors.center_color':'#b789e4','colors.edge_color':'#8a5abc','colors.pattern_color':'#5b10ab'}}],
 ['pattern',{id:'pattern',name:'Pattern',attribute_id:'tg.pattern',meta:{}}]
]);
const root=createRoot(document.getElementById('fixture'));
window.mount=(large=false)=>root.render(React.createElement(GiftPreview,{order,options,fallback:'G',large},
 React.createElement('span',{className:'venue-icon',style:{position:'relative',zIndex:4}},React.createElement(VenueIcon,{name:'MRKT'}))));
window.unmount=()=>root.render(null);
window.mountSelection=()=>{
 const collections=[{id:'gift',name:'Gift',attributes:{'tg.model':{allowed_values:['model']}}}];
 root.render(React.createElement(React.Fragment,null,
  React.createElement(CollectionRail,{collections,selected:'gift',select:()=>{}}),
  React.createElement(TraitPreview,{collection:collections[0],options,filters:{'tg.model':{op:'eq',value:{type:'enum',value:'model'}}},reset:()=>{}})));
};
window.mount();
</script></body></html>`;
const server = await createServer({
  root,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "preview-fixture",
      configureServer(server) {
        server.middlewares.use("/__preview_test", async (req, res, next) => {
          try {
            res.setHeader("Content-Type", "text/html");
            res.end(await server.transformIndexHtml("/__preview_test", html));
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
  const page = await browser.newPage({ viewport: { width: 900, height: 700 } });
  page.setDefaultTimeout(10000);
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript(() => {
    const clear = CanvasRenderingContext2D.prototype.clearRect;
    CanvasRenderingContext2D.prototype.clearRect = function (...args) {
      if (window.__failGift && this.canvas.closest(".model-animation"))
        throw Error("Injected renderer failure");
      return clear.apply(this, args);
    };
  });
  let animations = 0,
    patterns = 0;
  const tgs = gzipSync(
    JSON.stringify({
      v: "5.7.4",
      w: 512,
      h: 512,
      fr: 60,
      ip: 0,
      op: 180,
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
            p: {
              a: 1,
              k: [
                {
                  t: 0,
                  s: [128, 256, 0],
                  e: [384, 256, 0],
                  o: { x: 0.33, y: 0 },
                  i: { x: 0.67, y: 1 },
                },
                { t: 179, s: [384, 256, 0] },
              ],
            },
            a: { a: 0, k: [32, 32, 0] },
            s: { a: 0, k: [100, 100, 100] },
          },
          ao: 0,
          sw: 64,
          sh: 64,
          sc: "#ffffff",
          ip: 0,
          op: 180,
          st: 0,
          bm: 0,
        },
      ],
      assets: [],
    }),
  );
  const png = await page.evaluate(() => {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 16;
    const ctx = canvas.getContext("2d");
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(2, 2, 12, 12);
    return canvas.toDataURL("image/png").split(",")[1];
  });
  const image = Buffer.from(png, "base64");
  await page.route("**/media/**", async (route) => {
    const url = route.request().url();
    if (url.endsWith("/animation")) {
      animations++;
      await route.fulfill({
        contentType: "application/x-tgsticker",
        body: tgs,
      });
    } else {
      if (url.includes("/patterns/")) patterns++;
      await route.fulfill({ contentType: "image/png", body: image });
    }
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__preview_test`,
  );
  const preview = page.locator(".gift-preview");
  await preview.locator("img.loaded").waitFor();
  const logoVisible = () =>
    preview
      .locator(".venue-logo")
      .evaluate(
        (el) =>
          el.complete &&
          el.naturalWidth > 0 &&
          getComputedStyle(el).opacity === "1" &&
          getComputedStyle(el).position === "static",
      );
  await page.waitForFunction(
    () => document.querySelector(".venue-logo")?.naturalWidth > 0,
  );
  assert.equal(
    await logoVisible(),
    true,
    "Thumbnail styles do not hide or stretch marketplace logos",
  );
  assert.equal(animations, 0, "list thumbnails must not preload TGS");
  await preview.hover();
  await preview.locator(".model-animation.ready canvas").waitFor();
  assert.equal(
    await logoVisible(),
    true,
    "Animation playback hides only the model thumbnail, preserving the MRKT logo",
  );
  await page.evaluate(() => {
    window.__firstCanvas = document.querySelector(".model-animation canvas");
  });
  await page.mouse.move(800, 20);
  await page.waitForFunction(
    () => !document.querySelector(".model-animation.ready"),
  );
  assert.equal(
    await preview
      .locator(".gift-model-image")
      .evaluate((el) => getComputedStyle(el).opacity),
    "1",
  );
  await preview.hover();
  await preview.locator(".model-animation.ready canvas").waitFor();
  assert.equal(
    await page.evaluate(
      () =>
        window.__firstCanvas ===
        document.querySelector(".model-animation canvas"),
    ),
    true,
    "quick hover reuses renderer",
  );
  await page.evaluate(() => {
    window.__failGift = true;
  });
  await page.waitForFunction(
    () => !document.querySelector(".model-animation.ready"),
  );
  assert.equal(
    await preview
      .locator(".gift-model-image")
      .evaluate((el) => getComputedStyle(el).opacity),
    "1",
    "render failures restore the thumbnail",
  );
  assert.equal(await preview.locator(".model-animation canvas").count(), 0);
  await page.evaluate(() => {
    window.__failGift = false;
  });
  await page.mouse.move(800, 20);
  await preview.hover();
  await preview.locator(".model-animation.ready canvas").waitFor();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.waitForFunction(
    () => !document.querySelector(".model-animation canvas"),
  );
  assert.equal(
    await preview
      .locator(".gift-model-image")
      .evaluate((el) => getComputedStyle(el).opacity),
    "1",
  );
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await preview.locator(".model-animation.ready canvas").waitFor();
  await page.evaluate(() => {
    window.__hidden = true;
    Object.defineProperty(document, "hidden", {
      configurable: true,
      get: () => window.__hidden,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForFunction(
    () => !document.querySelector(".model-animation canvas"),
  );
  await page.evaluate(() => {
    window.__hidden = false;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await preview.locator(".model-animation.ready canvas").waitFor();
  await page.evaluate(() => window.scrollTo(0, 2000));
  await page.waitForFunction(
    () => !document.querySelector(".model-animation canvas"),
  );
  await page.evaluate(() => {
    window.scrollTo(0, 0);
    window.mount(true);
  });
  await preview.locator(".model-animation.ready canvas").waitFor();
  await page.evaluate(() => window.unmount());
  assert.equal(await page.locator(".model-animation canvas").count(), 0);
  await page.evaluate(() => window.mountSelection());
  await page.locator(".collection-art .model-animation.ready canvas").waitFor();
  await page.locator(".preview-model .model-animation.ready canvas").waitFor();
  assert.equal(
    await page.locator(".collection-tile .gift-preview[tabindex]").count(),
    0,
    "Collection buttons do not contain extra focus stops",
  );
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.waitForFunction(
    () => !document.querySelector(".model-animation canvas"),
  );
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.locator(".preview-model .model-animation.ready canvas").waitFor();
  await page.evaluate(() => window.scrollTo(0, 2000));
  await page.waitForFunction(
    () => !document.querySelector(".model-animation canvas"),
  );
  await page.evaluate(() => window.unmount());
  assert.equal(
    animations,
    1,
    "repeated playback shares downloaded and decoded TGS",
  );
  assert.equal(patterns, 1, "patterns share their decoded still");
  assert.deepEqual(errors, []);
  console.log(
    "Preview browser lifecycle passed: hover reuse, runtime error fallback, motion, visibility, offscreen, collection/selection autoplay, details and unmount.",
  );
} finally {
  await browser?.close();
  await server.close();
}
