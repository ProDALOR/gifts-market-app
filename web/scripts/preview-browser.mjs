// Isolated browser regression: real React/Lottie, synthetic public media, no hub or Telegram.
import assert from "node:assert/strict";
import { gzipSync, gunzipSync } from "node:zlib";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const root = fileURLToPath(new URL("..", import.meta.url));
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body>
<style>.detail-test{position:fixed;top:0;left:0;width:160px;height:160px;z-index:10}.loading-offscreen{margin-top:1800px}#fixture{margin:50px;height:4000px}.mixed-renderers{display:flex;flex-wrap:wrap;width:320px}</style><div id="fixture"></div>
<script type="module">
import React from 'react';
import {createRoot} from 'react-dom/client';
import {GiftPreview} from '/src/previews/gift-preview.tsx';
import {GiftArtwork} from '/src/previews/gift-artwork.tsx';
import { CollectionRail } from "/src/collections/collection-picker.tsx";
import { TraitPreview } from "/src/previews/trait-preview.tsx";
import { VenueIcon } from "/src/ui/icons.tsx";
import { SortPicker } from "/src/ui/pickers.tsx";
const order={target:{kind:'gift',id:'gift-1',number:1,traits:{model:'model',backdrop:'backdrop',pattern:'pattern'}}};
const options=new Map([
 ['model',{kind:'model',id:'model',name:'Model',media:{thumbnail_url:'/media/models/model/thumbnail',preview_url:'/media/models/model/preview',animation_url:'/media/models/model/animation'}}],
 ['backdrop',{kind:'backdrop',id:'backdrop',name:'Backdrop',palette:{center:'#b789e4',edge:'#8a5abc',pattern:'#5b10ab'}}],
 ['pattern',{kind:'pattern',id:'pattern',name:'Pattern',media:{thumbnail_url:'/media/patterns/pattern/thumbnail',animation_url:null}}]
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
const bulk=Array.from({length:24},(_,i)=>({kind:'model',id:'bulk-'+i,name:'Bulk '+String(i).padStart(2,'0'),media:{thumbnail_url:'/media/models/bulk-'+i+'/thumbnail',animation_url:'/media/models/bulk-'+i+'/animation'}}));
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
window.mountRail=()=>{const collections=Array.from({length:124},(_,i)=>({id:'rail-'+i,name:'Rail '+i,preview:{thumbnail_url:'/media/models/rail-'+i+'/thumbnail',animation_url:'/media/models/rail-'+i+'/animation'},model_ids:['rail-'+i],backdrop_ids:[],pattern_ids:[]}));root.render(React.createElement('div',{className:'workspace'},React.createElement(CollectionRail,{collections,selected:'rail-123',select:reset})));};
function Desktop() {
 const [collections,setCollections]=React.useState(()=>Array.from({length:124},(_,i)=>({id:'rail-'+i,name:i===123?'Intelligence Cup':'Rail '+i,preview:{thumbnail_url:'/media/models/rail-'+i+'/thumbnail',animation_url:'/media/models/rail-'+i+'/animation'},model_ids:[],backdrop_ids:[],pattern_ids:[]})));
 const [selected,setSelected]=React.useState('rail-123');
 const [sort,setSort]=React.useState('price');
 window.renameRail=name=>setCollections(items=>items.map(item=>item.id===selected?{...item,name}:item));
 return React.createElement('div',{className:'workspace'},React.createElement('main',{className:'orderbook market-shell'},
  React.createElement('aside',{className:'market-sidebar','data-scroll-container':true},
   React.createElement(CollectionRail,{collections,selected,select:setSelected}),
   React.createElement('div',{style:{height:500}},'More controls'),
   React.createElement('div',{className:'market-toolbar'},React.createElement('div',{className:'sort-control'},React.createElement(SortPicker,{value:sort,change:setSort}))),
   React.createElement('div',{style:{height:400}},'Additional controls')),
  React.createElement('div',{className:'market-results',style:{height:2000}},'Listings')));
}
window.mountDesktop=()=>root.render(React.createElement(Desktop));
window.mountReference=(animated=true)=>root.render(React.createElement(GiftArtwork,{media:{thumbnail_url:'/media/custom-still?v=1',animation_url:animated?'/media/custom-motion.tgs?v=2':null},fallback:'R',playback:'detail'}));
window.mountMixed=()=>root.render(React.createElement('div',{className:'mixed-renderers'},...Array.from({length:12},(_,index)=>React.createElement(GiftArtwork,{key:index,media:index%2?{thumbnail_url:'/media/gradient-still',preview_url:'/media/opacity-test/preview',animation_url:'/media/opacity-test/animation'}:options.get('model').media,fallback:'M',playback:'selected'}))));
window.mountGradient=(playback='detail')=>root.render(React.createElement(GiftArtwork,{media:{thumbnail_url:'/media/gradient-still',preview_url:'/media/opacity-test/preview',animation_url:'/media/opacity-test/animation'},fallback:'G',playback}));
window.mountSource=(thumbnail,playback='static')=>root.render(React.createElement(GiftArtwork,{media:{thumbnail_url:thumbnail,animation_url:'/media/custom-motion.tgs?v=2'},fallback:'F',playback}));
window.mountLarge=(playback='static',preview='/media/large-preview.svg')=>root.render(React.createElement(GiftArtwork,{media:{thumbnail_url:'/media/large-still',preview_url:preview,animation_url:'/media/custom-motion.tgs?v=2'},fallback:'L',size:'large',playback}));
window.mountLoading=(name='slow')=>root.render(React.createElement('div',{className:name==='offscreen'?'loading-offscreen':''},React.createElement(GiftArtwork,{media:{thumbnail_url:'/media/models/loading-'+name+'/thumbnail',placeholder_url:'/media/models/loading-'+name+'/placeholder'},fallback:'P',playback:'static',className:'loading-test'})));
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
    window.previewResizeTargets = 0;
    const NativeResizeObserver = window.ResizeObserver;
    window.ResizeObserver = class extends NativeResizeObserver {
      targets = new Set();
      observe(target, options) {
        if (!this.targets.has(target)) window.previewResizeTargets++;
        this.targets.add(target);
        super.observe(target, options);
      }
      unobserve(target) {
        if (this.targets.delete(target)) window.previewResizeTargets--;
        super.unobserve(target);
      }
      disconnect() {
        window.previewResizeTargets -= this.targets.size;
        this.targets.clear();
        super.disconnect();
      }
    };
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
  let transientPreviews = 0;
  const loadingStills = [];
  await page.route("**/media/**", async (route) => {
    const url = route.request().url();
    mediaRequests.push(new URL(url).pathname + new URL(url).search);
    if (url.includes("/loading-") && url.endsWith("/placeholder")) {
      if (url.includes("/loading-missing/"))
        await route.fulfill({ status: 404, body: "No outline" });
      else
        await route.fulfill({
          contentType: "image/svg+xml",
          headers: { "Cache-Control": "public,max-age=86400" },
          body: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path fill="#fff" d="M128,128L384,128L384,384L128,384z"/></svg>',
        });
      return;
    }
    if (
      url.includes("/loading-") &&
      url.endsWith("/thumbnail") &&
      !url.includes("/loading-fast/")
    ) {
      await new Promise((resolve) => loadingStills.push(resolve));
    }
    if (url.endsWith("/broken-still")) {
      await route.fulfill({ status: 404, body: "Missing" });
      return;
    }
    if (url.includes("/invalid-large/")) {
      await route.fulfill({ contentType: "image/svg+xml", body: "<invalid" });
      return;
    }
    if (url.includes("/missing-large/")) {
      await route.fulfill({ status: 404, body: "Missing animation" });
      return;
    }
    if (url.includes("/transient-large/") && ++transientPreviews <= 2) {
      await route.fulfill({
        status: 503,
        headers: { "Retry-After": "1" },
        body: "Busy",
      });
      return;
    }
    if (url.endsWith("/preview") || url.endsWith("/large-preview.svg")) {
      await route.fulfill({
        contentType: "image/svg+xml",
        body: '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><path d="M96 224h64v64h-64z" fill="#ffffff"/></svg>',
      });
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
  await preview.locator(".model-animation.ready > svg").waitFor();
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
  await page.waitForFunction(() =>
    [...document.querySelectorAll(".preview-model canvas")].every(
      (canvas) =>
        canvas.width === canvas.clientWidth * Math.min(devicePixelRatio, 2),
    ),
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
  await page.locator(".detail-test .model-animation.ready > svg").waitFor();
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
    () =>
      !document.querySelector(".model-animation canvas,.model-animation > svg"),
  );
  await page
    .getByRole("article", {
      name: "Preview strategy calculation guide",
      exact: true,
    })
    .waitFor();
  await page.keyboard.press("Escape");
  await page.locator(".valuation-popup").waitFor({ state: "detached" });
  await page.locator(".detail-test .model-animation.ready > svg").waitFor();
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
  // A narrow desktop sidebar must refit labels and retain cached artwork.
  await page.setViewportSize({ width: 390, height: 700 });
  await page.evaluate(() => window.mountDesktop());
  const selectedTile = page.locator('.collection-tile[aria-pressed="true"]');
  await selectedTile.locator(".model-animation.ready canvas").waitFor();
  const mobileFont = await selectedTile.locator("b").evaluate((label) => {
    window.selectedImage = label.parentElement.querySelector("img");
    window.selectedCanvas = label.parentElement.querySelector("canvas");
    return parseFloat(getComputedStyle(label).fontSize);
  });
  const beforeDesktop = animations;
  await page.setViewportSize({ width: 1440, height: 700 });
  await page.waitForFunction(
    (mobileFont) =>
      parseFloat(
        getComputedStyle(
          document.querySelector('.collection-tile[aria-pressed="true"] b'),
        ).fontSize,
      ) < mobileFont,
    mobileFont,
  );
  assert.equal(
    await selectedTile.evaluate(
      (tile) =>
        tile.querySelector("img") === window.selectedImage &&
        tile.querySelector("canvas") === window.selectedCanvas,
    ),
    true,
    "Desktop reflow retains collection artwork and its player",
  );
  assert.equal(
    animations,
    beforeDesktop,
    "Resizing reuses downloaded animation data",
  );
  await page.waitForFunction(() => {
    const tile = document.querySelector(
      '.collection-tile[aria-pressed="true"]',
    );
    const list = tile.parentElement.getBoundingClientRect();
    const bounds = tile.getBoundingClientRect();
    return (
      Math.abs(bounds.left - list.left - 67) < 1 &&
      bounds.right <= list.right + 1
    );
  });
  await page.evaluate(() => window.renameRail("Artisan Brick"));
  await page.waitForFunction(
    () =>
      parseFloat(
        getComputedStyle(
          document.querySelector('.collection-tile[aria-pressed="true"] b'),
        ).fontSize,
      ) === 16,
  );
  await page
    .getByRole("button", { name: "Search collections", exact: true })
    .click();
  await page.getByLabel("Filter collections").fill("Artisan");
  await page.waitForFunction(
    () =>
      getComputedStyle(
        document.querySelector('.collection-tile[aria-pressed="true"] b'),
      ).width === "56px",
  );
  await page.waitForFunction(() => {
    const canvas = document.querySelector(
      '.collection-tile[aria-pressed="true"] canvas',
    );
    return canvas && canvas.width === 32 * Math.min(devicePixelRatio, 2);
  });
  await page.getByLabel("Filter collections").press("Escape");
  await page.waitForFunction(() => {
    const canvas = document.querySelector(
      '.collection-tile[aria-pressed="true"] canvas',
    );
    return canvas && canvas.width === 55 * Math.min(devicePixelRatio, 2);
  });
  await page.setViewportSize({ width: 1024, height: 400 });
  const sidebar = page.locator(".market-sidebar");
  await sidebar.evaluate((node) => {
    node.scrollTop = 400;
  });
  await page.waitForFunction(
    () => !document.querySelector(".collection-art canvas"),
  );
  await page.getByRole("combobox", { name: "Sort by", exact: true }).click();
  const menu = page.getByRole("listbox", { name: "Sort by options" });
  const menuContained = () =>
    menu.evaluate((node) => {
      const bounds = node.getBoundingClientRect();
      const sidebar = node.closest(".market-sidebar").getBoundingClientRect();
      return (
        bounds.top >= sidebar.top &&
        bounds.bottom <= sidebar.bottom &&
        node.clientHeight > 0
      );
    });
  assert.equal(await menuContained(), true, "Dropdown fits the short sidebar");
  const menuNode = await menu.evaluate((node) => {
    window.sortMenu = node;
    return node.style.top;
  });
  await sidebar.evaluate((node) => {
    node.scrollTop = 515;
  });
  await page.waitForFunction(
    () => document.querySelector('[role="listbox"]').style.top !== "auto",
  );
  assert.equal(
    menuNode,
    "auto",
    "The menu initially opens above the lower trigger",
  );
  assert.equal(
    await menuContained(),
    true,
    "Scrolling places the same menu below an upper trigger",
  );
  assert.equal(await menu.evaluate((node) => node === window.sortMenu), true);
  assert.equal(
    await page.evaluate(() => window.scrollY),
    0,
    "Sidebar menus never scroll listings",
  );
  await page.setViewportSize({ width: 960, height: 320 });
  assert.equal(
    await menuContained(),
    true,
    "An open dropdown follows resizing",
  );
  await sidebar.evaluate((node) => {
    node.scrollTop = node.scrollHeight;
  });
  await menu.waitFor({ state: "detached" });
  assert.equal(
    await page.evaluate(() => window.scrollY),
    0,
    "A clipped trigger closes its menu without moving listings",
  );
  await sidebar.evaluate((node) => {
    node.scrollTop = 0;
  });
  await selectedTile.locator(".model-animation.ready canvas").waitFor();
  assert.ok((await page.locator(".model-animation canvas").count()) <= 6);
  await page.setViewportSize({ width: 480, height: 700 });
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
  await page.locator(".model-animation.ready > svg").waitFor();
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
  await page.locator(".model-animation.ready > svg").waitFor();
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
  await page.locator('img.loaded[src="/media/opacity-test/preview"]').waitFor();
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
  const beforeLarge = animations;
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.evaluate(() => {
    window.scrollTo(0, 0);
    window.mountLarge();
  });
  const still = page.locator(
    '.gift-preview-large img.loaded[src="/media/large-preview.svg"]',
  );
  await still.waitFor();
  assert.equal(
    await page.locator(".model-animation").count(),
    0,
    "Large static artwork uses an image without allocating a paused player",
  );
  assert.equal(
    await still.evaluate((image) => image.naturalWidth),
    512,
    "The native SVG replaces the low-resolution thumbnail",
  );
  await page.locator(".gift-preview-large").evaluate((element) => {
    element.style.width = element.style.height = "480px";
  });
  assert.equal(
    await still.evaluate((image) => image.getBoundingClientRect().width),
    478,
  );
  assert.equal(
    animations,
    beforeLarge,
    "Static previews do not download TGS to the browser",
  );
  await page.evaluate(() => window.mountLarge("interaction"));
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.mouse.move(899, 20);
  await still.waitFor();
  await page.locator(".gift-preview-large").hover();
  await page
    .locator(".gift-preview-large .model-animation.ready > svg")
    .waitFor();
  await page.mouse.move(899, 20);
  await page.waitForFunction(
    () => !document.querySelector(".model-animation.ready"),
  );
  assert.equal(
    await still.evaluate((image) => getComputedStyle(image).opacity),
    "1",
    "Leaving playback restores the native image",
  );
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.waitForFunction(
    () => !document.querySelector(".model-animation > svg"),
  );
  assert.equal(
    animations,
    beforeLarge,
    "Playback shares the already cached TGS",
  );
  await page.evaluate(() =>
    window.mountLarge("static", "/media/transient-large/preview"),
  );
  await page.locator('img.loaded[src="/media/large-still"]').waitFor();
  await page
    .locator('img.loaded[src="/media/transient-large/preview"]')
    .waitFor();
  assert.ok(
    transientPreviews >= 3 && transientPreviews <= 4,
    "SVG recovers after bounded transient retries",
  );
  assert.equal(
    await page.locator(".model-animation").count(),
    0,
    "SVG recovery does not allocate a player",
  );
  await page.evaluate(() =>
    window.mountLarge("static", "/media/invalid-large/preview"),
  );
  await page.locator('img.loaded[src="/media/large-still"]').waitFor();
  await page.waitForFunction(
    () => !document.querySelector('img[src="/media/invalid-large/preview"]'),
  );
  await page.waitForTimeout(150);
  const invalidAttempts = mediaRequests.filter(
    (url) => url === "/media/invalid-large/preview",
  ).length;
  assert.ok(
    invalidAttempts >= 2 && invalidAttempts <= 3,
    "Failed SVG decode has bounded recovery",
  );
  await page.waitForTimeout(200);
  assert.equal(
    mediaRequests.filter((url) => url === "/media/invalid-large/preview")
      .length,
    invalidAttempts,
    "Invalid SVG does not cause a remount/recovery loop",
  );
  await page.evaluate(() =>
    window.mountLarge("static", "/media/missing-large/preview"),
  );
  await page.locator('img.loaded[src="/media/large-still"]').waitFor();
  assert.equal(
    await page.locator(".model-animation").count(),
    0,
    "A failed SVG retains the thumbnail without a player",
  );
  await page.evaluate(() => window.unmount());
  assert.equal(await page.locator(".model-animation").count(), 0);
  await page.evaluate(() => {
    window.scrollTo(0, 0);
    window.mountLoading("slow");
  });
  await page.locator(".gift-model-placeholder").waitFor();
  assert.ok(
    await page
      .locator(".gift-model-placeholder clipPath path")
      .getAttribute("d"),
  );
  assert.equal(
    await page
      .locator(".gift-placeholder-shimmer")
      .evaluate((node) => getComputedStyle(node).animationName),
    "none",
    "Reduced motion keeps the silhouette static",
  );
  await page.emulateMedia({ reducedMotion: "no-preference" });
  assert.equal(
    await page
      .locator(".gift-placeholder-shimmer")
      .evaluate((node) => getComputedStyle(node).animationName),
    "gift-placeholder-shimmer",
  );
  assert.equal(
    await page.locator(".model-animation").count(),
    0,
    "Silhouettes never allocate a player",
  );
  loadingStills.splice(0).forEach((release) => release());
  await page.locator(".loading-test img.loaded").waitFor();
  assert.equal(
    await page.locator(".gift-model-placeholder").count(),
    0,
    "Loaded stills remove the shimmer",
  );
  await page.evaluate(() => window.mountLoading("fast"));
  await page.locator(".loading-test img.loaded").waitFor();
  await page.waitForTimeout(180);
  assert.equal(
    mediaRequests.filter((url) => url.includes("/loading-fast/placeholder"))
      .length,
    0,
    "Fast stills skip outline requests",
  );
  await page.evaluate(() => window.mountLoading("missing"));
  await page.waitForFunction(() =>
    document.querySelector(".loading-test .gift-loading-placeholder"),
  );
  await page.waitForTimeout(250);
  assert.equal(
    await page.locator(".gift-model-placeholder").count(),
    0,
    "Absent outlines keep the local silhouette",
  );
  assert.equal(
    mediaRequests.filter((url) => url.includes("/loading-missing/placeholder"))
      .length,
    1,
    "Missing outlines are not repeatedly requested",
  );
  loadingStills.splice(0).forEach((release) => release());
  await page.locator(".loading-test img.loaded").waitFor();
  await page.evaluate(() => window.mountLoading("offscreen"));
  await page.waitForTimeout(180);
  assert.equal(
    mediaRequests.filter((url) =>
      url.includes("/loading-offscreen/placeholder"),
    ).length,
    0,
    "Offscreen artwork skips outlines",
  );
  loadingStills.splice(0).forEach((release) => release());
  await page.evaluate(() => window.unmount());
  await page.waitForFunction(() => window.previewResizeTargets === 0);
  assert.deepEqual(errors, []);
  console.log(
    "Preview browser lifecycle passed: hover reuse, runtime error fallback, motion, visibility, offscreen, collection/selection autoplay, details, sharp large stills, SVG retry/fallback, transparent CSS placeholders, bounded downloads/playback, responsive canvas sizes, desktop labels/menus, long-rail initialization and observer cleanup.",
  );
} finally {
  await browser?.close();
  await server.close();
}
