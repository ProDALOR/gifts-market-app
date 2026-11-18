// Isolated browser regression: real React/Lottie, synthetic public media, no hub or Telegram.
import assert from "node:assert/strict";
import { gzipSync, gunzipSync } from "node:zlib";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const root = fileURLToPath(new URL("..", import.meta.url));
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body>
<style>.detail-test{position:fixed;top:0;left:0;width:160px;height:160px;z-index:10}#fixture{margin:50px;height:4000px}.mixed-renderers{display:flex;flex-wrap:wrap;width:320px}</style><div id="fixture"></div>
<script type="module">
import React from 'react';
import {createRoot} from 'react-dom/client';
import {GiftPreview} from '/src/previews/gift-preview.tsx';
import {GiftArtwork} from '/src/previews/gift-artwork.tsx';
import { CollectionRail } from "/src/collections/collection-picker.tsx";
import { TraitPreview } from "/src/previews/trait-preview.tsx";
import { VenueIcon } from "/src/ui/icons.tsx";
const order={target:{kind:'gift',gift:{id:'gift-1',collection_id:'gift',number:1,traits:{model:'model',backdrop:'backdrop',pattern:'pattern'}}}};
const options=new Map([
 ['model',{kind:'model',id:'model',name:'Model',media:{thumbnail_url:'/media/models/model',animation_url:'/media/models/model/animation'}}],
 ['backdrop',{kind:'backdrop',id:'backdrop',name:'Backdrop',palette:{center:'#b789e4',edge:'#8a5abc',pattern:'#5b10ab'}}],
 ['pattern',{kind:'pattern',id:'pattern',name:'Pattern',media:{thumbnail_url:'/media/patterns/pattern',animation_url:null}}]
]);
import {loadAnimationData} from '/src/previews/animation-data.ts';
import {useBackgroundPlaybackSuspension} from '/src/previews/use-playback.ts';
import {ValuationInfo} from '/src/orderbook/valuation-info.tsx';
const valuationState={key:'preview',fields:{primary:[],secondary:[]},loading:false,
 report:{strategy:'preview',sync:{state:'live'},evaluated_at:'2026-11-18T12:00:00Z',display:{note:'Fixture estimate'}}};
const root=createRoot(document.getElementById('fixture'));
window.mount=(large=false)=>root.render(React.createElement(GiftPreview,{order,options,fallback:'G',size:large?'large':'small',playback:large?'detail':'interaction'},
 React.createElement('span',{className:'venue-icon',style:{position:'relative',zIndex:4}},React.createElement(VenueIcon,{name:'MRKT'}))));
window.unmount=()=>root.render(null);
window.mountSelection=()=>{
 const collections=[{id:'gift',name:'Gift',preview:options.get('model').media,model_ids:['model'],backdrop_ids:[],pattern_ids:[]}];
 root.render(React.createElement(React.Fragment,null,
  React.createElement(CollectionRail,{collections,selected:'gift',select:()=>{}}),
  React.createElement(TraitPreview,{collection:collections[0],options,filters:{models:['model']},reset:()=>{}})));
};
const bulk=Array.from({length:24},(_,i)=>({kind:'model',id:'bulk-'+i,name:'Bulk '+String(i).padStart(2,'0'),media:{thumbnail_url:'/media/models/bulk-'+i,animation_url:'/media/models/bulk-'+i+'/animation'}}));
const bulkOptions=new Map(bulk.map(item=>[item.id,item]));
const bulkCollection={id:'bulk',model_ids:bulk.map(item=>item.id),backdrop_ids:[],pattern_ids:[]};
const bulkFilters={models:bulk.map(item=>item.id)};
const reset=()=>{};
function Many({detail}) {
 useBackgroundPlaybackSuspension(detail);
 return React.createElement(React.Fragment,null,
  React.createElement('div',{className:'workspace'},React.createElement('main',{className:'orderbook'},React.createElement(TraitPreview,{collection:bulkCollection,options:bulkOptions,filters:bulkFilters,reset}))),
  detail?React.createElement(GiftArtwork,{media:options.get('model').media,fallback:'D',playback:'detail',className:'detail-test'}):null,
  detail?React.createElement(ValuationInfo,{state:valuationState}):null);
}
window.preload=()=>Promise.all(bulk.map(item=>loadAnimationData('/media/models/'+item.id+'/animation',new AbortController().signal)));
window.mountMany=(detail=false)=>{document.getElementById('fixture').style.margin='0';root.render(React.createElement(Many,{detail}));};
window.mountRail=()=>{const collections=Array.from({length:124},(_,i)=>({id:'rail-'+i,name:'Rail '+i,preview:{thumbnail_url:'/media/models/rail-'+i,animation_url:'/media/models/rail-'+i+'/animation'},model_ids:['rail-'+i],backdrop_ids:[],pattern_ids:[]}));root.render(React.createElement('div',{className:'workspace'},React.createElement(CollectionRail,{collections,selected:'rail-123',select:reset})));};
window.mountReference=(animated=true)=>root.render(React.createElement(GiftArtwork,{media:{thumbnail_url:'/media/custom-still?v=1',animation_url:animated?'/media/custom-motion.tgs?v=2':null},fallback:'R',playback:'detail'}));
window.mountMixed=()=>root.render(React.createElement('div',{className:'mixed-renderers'},...Array.from({length:12},(_,index)=>React.createElement(GiftArtwork,{key:index,media:index%2?{thumbnail_url:'/media/gradient-still',animation_url:'/media/opacity-test/animation'}:options.get('model').media,fallback:'M',playback:'selected'}))));
window.mountGradient=(playback='detail')=>root.render(React.createElement(GiftArtwork,{media:{thumbnail_url:'/media/gradient-still',animation_url:'/media/opacity-test/animation'},fallback:'G',playback}));
window.mountSource=(thumbnail,playback='static')=>root.render(React.createElement(GiftArtwork,{media:{thumbnail_url:thumbnail,animation_url:'/media/custom-motion.tgs?v=2'},fallback:'F',playback}));
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
            res.setHeader(
              "Content-Security-Policy",
              "style-src-attr 'none'; style-src-elem 'self' 'unsafe-inline'",
            );
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
    const timeout = window.setTimeout;
    window.setTimeout = function (callback, delay, ...args) {
      if (window.__trackPreviewTimers && [100, 150, 180].includes(delay))
        window.__previewTimers++;
      return timeout(callback, delay, ...args);
    };
    const clear = CanvasRenderingContext2D.prototype.clearRect;
    CanvasRenderingContext2D.prototype.clearRect = function (...args) {
      if (window.__failGift && this.canvas.closest(".model-animation"))
        throw Error("Injected renderer failure");
      return clear.apply(this, args);
    };
  });
  const mediaRequests = [];
  let animations = 0,
    patterns = 0,
    activeDownloads = 0,
    peakDownloads = 0;
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
  const gradient = JSON.parse(gunzipSync(tgs));
  gradient.layers = [
    {
      ddd: 0,
      ind: 1,
      ty: 4,
      nm: "Independent opacity",
      ks: {
        o: { a: 0, k: 100 },
        r: { a: 0, k: 0 },
        p: { a: 0, k: [0, 0, 0] },
        a: { a: 0, k: [0, 0, 0] },
        s: { a: 0, k: [100, 100, 100] },
      },
      shapes: [
        {
          ty: "rc",
          d: 1,
          s: { a: 0, k: [512, 512] },
          p: { a: 0, k: [256, 256] },
          r: { a: 0, k: 0 },
        },
        {
          ty: "gf",
          o: { a: 0, k: 100 },
          s: { a: 0, k: [256, 256] },
          e: { a: 0, k: [400, 256] },
          t: 2,
          h: { a: 0, k: 0 },
          a: { a: 0, k: 0 },
          r: 1,
          g: {
            p: 3,
            k: {
              a: 0,
              k: [0, 1, 1, 1, 0.8, 1, 1, 1, 1, 1, 1, 1, 0, 1, 0.5, 0.5, 1, 0],
            },
          },
        },
      ],
      ip: 0,
      op: 180,
      st: 0,
      bm: 0,
    },
  ];
  const gradientTgs = gzipSync(JSON.stringify(gradient));
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
    mediaRequests.push(new URL(url).pathname + new URL(url).search);
    if (url.endsWith("/broken-still")) {
      await route.fulfill({ status: 404, body: "Missing" });
      return;
    }
    if (
      url.endsWith("/animation") ||
      new URL(url).pathname === "/media/custom-motion.tgs"
    ) {
      animations++;
      peakDownloads = Math.max(peakDownloads, ++activeDownloads);
      if (url.includes("/bulk-"))
        await new Promise((resolve) => setTimeout(resolve, 50));
      try {
        await route.fulfill({
          contentType: "application/x-tgsticker",
          body: url.includes("/opacity-test/") ? gradientTgs : tgs,
        });
      } finally {
        activeDownloads--;
      }
    } else {
      if (url.includes("/patterns/")) patterns++;
      await route.fulfill({ contentType: "image/png", body: image });
    }
  });
  let metadataRequests = 0,
    guideRequests = 0;
  await page.route("**/valuations/strategies", (route) => {
    metadataRequests++;
    return route.fulfill({
      json: [
        {
          id: "preview",
          name: "Preview strategy",
          description: "Fixture strategy",
          documentation_url: "/__preview_guide.md",
          documentation_revision: "one",
        },
      ],
    });
  });
  await page.route("**/__preview_guide.md", (route) => {
    guideRequests++;
    return route.fulfill({
      contentType: "text/markdown",
      body: "# Preview calculation guide",
    });
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
    await preview
      .locator(".model-animation.ready")
      .evaluate((el) => getComputedStyle(el).animationName),
    "preview-handoff",
  );
  assert.equal(
    await preview
      .locator(".gift-model-image")
      .evaluate((el) => getComputedStyle(el).transitionDuration),
    "0.12s",
  );
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
  await page.evaluate(() => {
    window.scrollTo(0, 0);
    return window.preload();
  });
  assert.equal(
    animations,
    25,
    "All twenty-four distinct queued animations complete",
  );
  assert.ok(
    peakDownloads <= 4,
    "Concurrent downloads stay within the shared budget",
  );
  await page.setViewportSize({ width: 320, height: 844 });
  await page.evaluate(() => window.mountMany());
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".preview-model .model-animation.ready canvas")
        .length === 6,
  );
  await page.evaluate(() => {
    window.before = new Map(
      [...document.querySelectorAll(".preview-model")]
        .map((model) => [
          model.querySelector("span[title]").title,
          model.querySelector(".model-animation canvas"),
        ])
        .filter(([, canvas]) => canvas),
    );
  });
  await page.setViewportSize({ width: 480, height: 844 });
  await page.waitForFunction(
    () =>
      document
        .querySelector(".preview-model .gift-preview")
        .getBoundingClientRect().width === 40,
  );
  assert.equal(
    await page.evaluate(
      () =>
        [...document.querySelectorAll(".preview-model")].filter(
          (model) =>
            window.before.has(model.querySelector("span[title]").title) &&
            window.before.get(model.querySelector("span[title]").title) ===
              model.querySelector(".model-animation canvas"),
        ).length,
    ),
    6,
    "Reflow keeps every admitted player mounted",
  );
  await page.locator(".preview-model").last().locator(".gift-preview").hover();
  await page
    .locator(".preview-model")
    .last()
    .locator(".model-animation.ready canvas")
    .waitFor();
  assert.equal(
    await page.locator(".model-animation.ready canvas").count(),
    6,
    "Interacting with a waiting model preempts autoplay instead of exceeding the limit",
  );
  await page.mouse.move(479, 800);
  await page.evaluate(() => window.mountMany(true));
  await page.locator(".detail-test .model-animation.ready canvas").waitFor();
  assert.equal(
    await page.locator(".preview-model .model-animation.ready canvas").count(),
    0,
    "Opening details suspends background players",
  );
  const beforePopup = animations;
  assert.equal(metadataRequests, 0);
  await page
    .getByRole("button", { name: "How is this calculated?", exact: true })
    .click();
  await page.waitForFunction(
    () => !document.querySelector(".model-animation canvas"),
  );
  await page
    .getByRole("article", {
      name: "Preview strategy calculation guide",
      exact: true,
    })
    .waitFor();
  await page.keyboard.press("Escape");
  await page.locator(".valuation-popup").waitFor({ state: "detached" });
  await page.locator(".detail-test .model-animation.ready canvas").waitFor();
  assert.equal(
    animations,
    beforePopup,
    "Closing the info popup resumes detail playback from cached data",
  );
  await page
    .getByRole("button", { name: "How is this calculated?", exact: true })
    .click();
  await page
    .getByRole("article", {
      name: "Preview strategy calculation guide",
      exact: true,
    })
    .waitFor();
  assert.equal(
    metadataRequests,
    1,
    "Reopening the actual popup reuses discovery",
  );
  assert.equal(
    guideRequests,
    1,
    "Reopening the actual popup reuses its rendered guide",
  );
  await page.evaluate(() => {
    window.mountMany(false);
    window.scrollTo(0, 0);
  });
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".preview-model .model-animation.ready canvas")
        .length === 6,
  );
  await page.evaluate(() => window.mountRail());
  await page
    .locator(
      '.collection-tile[aria-pressed="true"] .model-animation.ready canvas',
    )
    .waitFor();
  assert.ok(
    animations - 25 <= 2,
    "Initial positioning skips transient animation downloads across the entire catalog",
  );
  assert.equal(await page.locator(".preview-model").count(), 0);
  await page.evaluate(() => window.unmount());
  const beforeReferences = animations;
  await page.evaluate(() => window.mountReference(false));
  await page.locator('img[src="/media/custom-still?v=1"]').waitFor();
  await page.waitForTimeout(300);
  assert.equal(
    animations,
    beforeReferences,
    "A null animation reference never starts playback or downloads",
  );
  assert.equal(await page.locator(".model-animation").count(), 0);
  await page.evaluate(() => window.mountReference(true));
  await page.locator(".model-animation.ready canvas").waitFor();
  assert.ok(
    mediaRequests.includes("/media/custom-motion.tgs?v=2"),
    "Use the supplied animation URL, including query parameters",
  );
  assert.ok(
    !mediaRequests.some(
      (url) => url.includes("custom-still") && url.includes("animation"),
    ),
    "Never append an animation suffix to a thumbnail URL",
  );
  await page.evaluate(() => window.mountReference(false));
  await page.waitForFunction(() => !document.querySelector(".model-animation"));
  await page.evaluate(() => {
    window.__previewTimers = 0;
    window.__trackPreviewTimers = true;
    window.mountSource("/media/static-source");
  });
  await page.locator('img.loaded[src="/media/static-source"]').waitFor();
  await page.locator(".gift-preview").hover();
  await page.waitForTimeout(220);
  assert.equal(
    await page.evaluate(() => window.__previewTimers),
    0,
    "Static artwork schedules neither hover nor settled autoplay timers",
  );
  assert.equal(await page.locator(".model-animation").count(), 0);
  await page.evaluate(() => {
    window.__trackPreviewTimers = false;
    window.mountSource("/media/static-source", "detail");
  });
  await page.locator(".model-animation.ready canvas").waitFor();
  const downloaded = animations;
  await page.evaluate(() =>
    window.mountSource("/media/static-source", "static"),
  );
  await page.waitForFunction(() => !document.querySelector(".model-animation"));
  assert.equal(
    animations,
    downloaded,
    "Changing to a static policy releases the player without another download",
  );
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.evaluate(() => {
    window.__previewTimers = 0;
    window.__trackPreviewTimers = true;
    window.mountSource("/media/reduced-motion-source", "selected");
  });
  await page
    .locator('img.loaded[src="/media/reduced-motion-source"]')
    .waitFor();
  await page.locator(".gift-preview").hover();
  await page.waitForTimeout(220);
  assert.equal(
    await page.evaluate(() => window.__previewTimers),
    0,
    "Reduced motion schedules no hover or autoplay timers",
  );
  await page.evaluate(() => {
    window.__trackPreviewTimers = false;
    window.mountSource("/media/broken-still");
  });
  await page.waitForFunction(
    () => !document.querySelector(".gift-model-image"),
  );
  await page.evaluate(() => window.mountSource("/media/recovered-source"));
  await page.locator('img.loaded[src="/media/recovered-source"]').waitFor();
  assert.equal(
    await page.locator(".gift-preview-fallback").count(),
    0,
    "Changing media resets a failed thumbnail without caller-owned keys",
  );
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.evaluate(() => {
    window.scrollTo(0, 0);
    window.mountGradient();
  });
  const svg = page.locator(".model-animation.ready > svg");
  await svg.waitFor();
  assert.equal(
    await page.locator(".model-animation canvas").count(),
    0,
    "Opacity gradients use the compatible SVG renderer",
  );
  assert.ok(
    (await svg.locator("mask").count()) > 0,
    "Independent opacity stops retain their mask",
  );
  const pixels = await svg.evaluate(async (element) => {
    const image = new Image();
    image.src =
      "data:image/svg+xml;charset=utf-8," +
      encodeURIComponent(new XMLSerializer().serializeToString(element));
    await image.decode();
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 512;
    const context = canvas.getContext("2d");
    context.drawImage(image, 0, 0, 512, 512);
    return {
      corner: [...context.getImageData(0, 0, 1, 1).data],
      center: [...context.getImageData(256, 256, 1, 1).data],
    };
  });
  assert.equal(
    pixels.corner[3],
    0,
    "No opaque white rectangle outside the gradient",
  );
  assert.ok(
    pixels.center[0] > 245 && pixels.center[3] > 240,
    "White gift details remain white and opaque",
  );
  const svgDownloads = animations;
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.waitForFunction(
    () => !document.querySelector(".model-animation > svg"),
  );
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await svg.waitFor();
  await page.evaluate(() => window.scrollTo(0, 2000));
  await page.waitForFunction(
    () => !document.querySelector(".model-animation > svg"),
  );
  await page.evaluate(() => window.scrollTo(0, 0));
  await svg.waitFor();
  assert.equal(
    animations,
    svgDownloads,
    "SVG playback shares the same decoded TGS cache",
  );
  await page.evaluate(() => window.mountGradient("static"));
  await page.waitForFunction(() => !document.querySelector(".model-animation"));
  await page.evaluate(() => window.mountMixed());
  await page.waitForFunction(
    () =>
      document.querySelectorAll(
        ".model-animation.ready > svg,.model-animation.ready > canvas",
      ).length === 6,
  );
  assert.ok((await page.locator(".model-animation.ready > svg").count()) > 0);
  assert.ok(
    (await page.locator(".model-animation.ready > canvas").count()) > 0,
  );
  const ids = await page
    .locator(".model-animation svg [id]")
    .evaluateAll((nodes) => nodes.map((node) => node.id));
  assert.equal(
    new Set(ids).size,
    ids.length,
    "Concurrent SVG players keep independent gradient and mask IDs",
  );
  assert.equal(
    animations,
    svgDownloads,
    "Mixed renderers share existing cached downloads",
  );
  await page.evaluate(() => window.unmount());
  assert.equal(await page.locator(".model-animation").count(), 0);
  assert.deepEqual(errors, []);
  console.log(
    "Preview browser lifecycle passed: hover reuse, runtime error fallback, motion, visibility, offscreen, collection/selection autoplay, details, bounded downloads/playback, stable reflow, long-rail initialization and unmount.",
  );
} finally {
  await browser?.close();
  await server.close();
}
