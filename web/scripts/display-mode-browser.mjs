// Real cards, collection picker and settings; synthetic media, no account or market services.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { createServer } from "vite";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const cacheDir = await mkdtemp(join(tmpdir(), "igifts-display-"));
const counts = new Map(),
  aborted = new Set();
let delayItems = false,
  png;
const svg =
  '<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128"><rect width="128" height="128" rx="24" fill="#eac2e1"/></svg>';
const animation = gzipSync(
  JSON.stringify({
    v: "5.7.4",
    w: 128,
    h: 128,
    fr: 30,
    ip: 0,
    op: 90,
    assets: [],
    layers: [
      {
        ddd: 0,
        ind: 1,
        ty: 1,
        nm: "Square",
        sr: 1,
        ks: {
          o: { a: 0, k: 100 },
          r: { a: 0, k: 0 },
          p: { a: 0, k: [64, 64, 0] },
          a: { a: 0, k: [32, 32, 0] },
          s: { a: 0, k: [100, 100, 100] },
        },
        ao: 0,
        sw: 64,
        sh: 64,
        sc: "#75506e",
        ip: 0,
        op: 90,
        st: 0,
        bm: 0,
      },
    ],
  }),
);
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module">
import React,{useState}from'react';import{createRoot}from'react-dom/client';import{flushSync}from'react-dom';
import{DisplaySettings}from'/src/profile/display-settings.tsx';import{GiftCardView}from'/src/previews/gift-card.tsx';import{GiftArtwork}from'/src/previews/gift-artwork.tsx';import{CollectionRail}from'/src/collections/collection-picker.tsx';import'/src/style.css';import'/src/profile/profile.css';
const h=React.createElement,media=id=>({thumbnail_url:'/media/models/'+id+'/thumbnail',preview_url:'/media/models/'+id+'/preview',animation_url:'/media/models/'+id+'/animation'}),options=new Map(['item','bid','pending'].map(id=>[id,{kind:'model',id,name:id,media:media(id)}]));
const collections=['a','b','c'].map(id=>({id,name:'Collection '+id,preview:media(id),model_ids:[],backdrop_ids:[],pattern_ids:[]}));
const gift={target:{kind:'gift',id:'one',number:1,traits:{model:'item'}}};
const bid={kind:'bid',target:{kind:'criteria',models:['bid'],backdrops:[],patterns:[]}};
function App(){const[selected,select]=useState('a'),[details,show]=useState(false),[pending,setPending]=useState(false),[detailId,setDetailId]=useState('detail');window.pendingCard=()=>setPending(true);window.failDetail=()=>setDetailId('missing');window.emptyDetail=()=>setDetailId(null);return h('main',{className:'workspace orderbook'},h(DisplaySettings),h(CollectionRail,{collections,selected,select}),h('ul',{className:'orders'},...[pending?{...gift,target:{...gift.target,traits:{model:'pending'}}}:gift,bid].map((subject,i)=>h(GiftCardView,{key:i,subject,options,collectionName:'Gift',identity:'#1',price:'12',currency:'GRAM',action:h('button',{className:'card-open','aria-label':i?'Open bid':'Open gift',onClick:()=>show(!details)})}))),details&&h('div',{id:'details',style:{height:180}},h(GiftArtwork,{media:detailId?media(detailId):undefined,size:'large',playback:'detail',fallback:'G'})));}
const root=createRoot(document.getElementById('root'));window.unmount=()=>root.unmount();window.remount=()=>{flushSync(()=>root.render(null));flushSync(()=>root.render(h(App)))};root.render(h(App));
</script><style>.orders{margin:12px 0}.orders .gift-card{min-height:130px}.collection-rail{margin-top:12px}#details .gift-preview{width:180px;height:180px}</style></body></html>`;
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "display-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url.startsWith("/__display")) {
            res.setHeader("Content-Type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
            return;
          }
          if (!req.url.startsWith("/media/")) return next();
          const path = req.url.split("?")[0];
          counts.set(path, (counts.get(path) || 0) + 1);
          if (path.includes("/missing/")) {
            res.statusCode = 404;
            res.end();
            return;
          }
          res.setHeader("Cache-Control", "public,max-age=86400");
          const motion = path.endsWith("/animation"),
            thumbnail = path.endsWith("/thumbnail");
          res.setHeader(
            "Content-Type",
            motion
              ? "application/x-tgsticker"
              : thumbnail
                ? "image/png"
                : "image/svg+xml",
          );
          const timer = setTimeout(
            () => res.end(motion ? animation : thumbnail ? png : svg),
            motion && path.includes("/pending/") && delayItems
              ? 1500
              : motion && path.includes("/detail/")
                ? 350
                : 10,
          );
          res.on("close", () => {
            clearTimeout(timer);
            if (!res.writableFinished) aborted.add(path);
          });
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
      const x = c.getContext("2d");
      x.fillStyle = "#75506e";
      x.fillRect(0, 0, 8, 8);
      return c.toDataURL().split(",")[1];
    }),
    "base64",
  );
  await encoder.close();
  const url = server.resolvedUrls.local[0] + "__display";
  for (const width of [390, 1440]) {
    counts.clear();
    aborted.clear();
    delayItems = false;
    const context = await browser.newContext({
        viewport: { width, height: 1000 },
        reducedMotion: "no-preference",
      }),
      page = await context.newPage(),
      errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto(url);
    await page
      .getByRole("radio", { name: "Performance", exact: true })
      .waitFor();
    assert.equal(
      await page
        .getByRole("radio", { name: "Performance", exact: true })
        .isChecked(),
      true,
    );
    await page.waitForFunction(
      () =>
        document.querySelectorAll(".gift-model-image.loaded").length >= 5 &&
        document.querySelectorAll(".collection-tile .model-animation.ready")
          .length === 1,
    );
    assert(
      [...counts.keys()]
        .filter((p) => p.endsWith("/animation"))
        .every((p) => p === "/media/models/a/animation"),
    );
    assert(
      ![...counts.keys()].some(
        (p) => p.startsWith("/media/models/") && p.endsWith("/preview"),
      ),
      "performance requests no SVG model previews",
    );
    assert.equal(await page.locator(".orders .model-animation").count(), 0);
    await page.locator(".orders .gift-card").first().hover();
    await page
      .locator(".orders .gift-card")
      .first()
      .locator(".model-animation.ready")
      .waitFor();
    assert.equal(
      counts.get("/media/models/item/animation"),
      1,
      "performance animates a hovered card",
    );
    await page.mouse.move(0, 0);
    await page.waitForFunction(
      () => !document.querySelector(".orders .model-animation.ready"),
    );
    await page.getByRole("button", { name: "Open gift", exact: true }).focus();
    await page
      .locator(".orders .gift-card")
      .first()
      .locator(".model-animation.ready")
      .waitFor();
    await page.getByRole("radio", { name: "Performance", exact: true }).focus();

    await page
      .getByRole("button", { name: "Collection b", exact: true })
      .click();
    await page.waitForFunction(
      () =>
        !!document.querySelector(
          '[data-collection-id="b"] .model-animation.ready',
        ),
    );
    assert.equal(
      await page.locator('[data-collection-id="a"] .model-animation').count(),
      0,
    );
    assert.equal(
      await page.locator('[data-collection-id="c"] .model-animation').count(),
      0,
    );
    await page.getByRole("button", { name: "Open gift", exact: true }).click();
    await page.locator("#details .gift-loading-placeholder").waitFor();
    await page.locator("#details .model-animation.ready").waitFor();
    assert.equal(
      counts.get("/media/models/detail/preview") || 0,
      0,
      "opened details request no SVG still",
    );
    assert.equal(
      counts.get("/media/models/detail/thumbnail") || 0,
      0,
      "opened details request no raster still",
    );
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.locator("#details .gift-loading-placeholder").waitFor();
    assert.equal(await page.locator("#details .gift-model-image").count(), 0);
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await page.locator("#details .model-animation.ready").waitFor();
    await page.getByRole("radio", { name: "Quality", exact: true }).check();
    await page.waitForFunction(() =>
      [...document.querySelectorAll(".orders .gift-model-image")].every(
        (i) => i.src.endsWith("/preview") && i.complete,
      ),
    );
    await page.waitForFunction(
      () =>
        document.querySelectorAll(".collection-tile .model-animation.ready")
          .length === 3,
    );
    delayItems = true;
    await page.evaluate(() => window.pendingCard());
    await page.locator(".orders .gift-card").first().hover();
    for (
      let n = 0;
      !counts.get("/media/models/pending/animation") && n < 100;
      n++
    )
      await page.waitForTimeout(20);
    assert(counts.get("/media/models/pending/animation"));
    await page.getByRole("radio", { name: "Performance", exact: true }).check();
    await page.waitForFunction(
      () => !document.querySelector(".orders .model-animation"),
    );
    for (
      let n = 0;
      !aborted.has("/media/models/pending/animation") && n < 50;
      n++
    )
      await page.waitForTimeout(10);
    assert(
      aborted.has("/media/models/pending/animation"),
      "mode switch cancels a pending card animation",
    );
    await page.locator("#details .model-animation.ready").waitFor();
    assert.equal(
      counts.get("/media/models/detail/animation"),
      1,
      "mode changes and motion preferences reuse TGS data",
    );
    await page.evaluate(() => window.failDetail());
    await page.locator("#details .gift-loading-placeholder").waitFor();
    for (
      let n = 0;
      !counts.get("/media/models/missing/animation") && n < 100;
      n++
    )
      await page.waitForTimeout(10);
    assert.equal(counts.get("/media/models/missing/animation"), 1);
    await page.waitForFunction(
      () => !document.querySelector("#details .gift-placeholder-shimmer"),
    );
    assert.equal(
      await page.locator("#details .gift-model-image").count(),
      0,
      "failed animations keep a placeholder instead of downloading a still",
    );
    await page.evaluate(() => window.emptyDetail());
    await page.locator("#details .gift-loading-placeholder").waitFor();
    assert.equal(
      await page.locator("#details .gift-preview-fallback").count(),
      0,
      "unresolved animations show the local placeholder, never a letter",
    );
    await page.reload();
    await page
      .getByRole("radio", { name: "Performance", exact: true })
      .waitFor();
    assert.equal(
      await page
        .getByRole("radio", { name: "Performance", exact: true })
        .isChecked(),
      true,
    );
    const other = await context.newPage();
    await other.goto(url);
    await other.getByRole("radio", { name: "Quality", exact: true }).check();
    await page.waitForFunction(
      () => document.querySelector('input[value="quality"]').checked,
    );
    await other.close();
    await page.getByRole("radio", { name: "Quality", exact: true }).focus();
    await page.keyboard.press("ArrowRight");
    assert.equal(
      await page
        .getByRole("radio", { name: "Performance", exact: true })
        .isChecked(),
      true,
      "native keyboard radio navigation switches mode",
    );
    assert.deepEqual(errors, []);
    for (const [path, count] of counts)
      if (path.endsWith("/thumbnail"))
        assert.equal(count, 1, `warm thumbnails reuse HTTP caching: ${path}`);
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
      true,
    );
    await context.close();
  }
  const blocked = await browser.newPage({
    viewport: { width: 390, height: 1000 },
    reducedMotion: "reduce",
  });
  await blocked.addInitScript(() =>
    Object.defineProperty(window, "localStorage", {
      get() {
        throw new Error("Storage disabled");
      },
    }),
  );
  await blocked.goto(url);
  await blocked.getByRole("radio", { name: "Quality", exact: true }).waitFor();
  assert.equal(
    await blocked
      .getByRole("radio", { name: "Quality", exact: true })
      .isChecked(),
    false,
  );
  await blocked.getByRole("radio", { name: "Quality", exact: true }).check();
  await blocked.waitForFunction(() =>
    [...document.querySelectorAll(".orders .gift-model-image")].every((i) =>
      i.src.endsWith("/preview"),
    ),
  );
  await blocked.close();
  const readOnly = await browser.newPage({
    viewport: { width: 390, height: 1000 },
    reducedMotion: "reduce",
  });
  await readOnly.addInitScript(() =>
    Object.defineProperty(Storage.prototype, "setItem", {
      value() {
        throw new Error("Storage is read-only");
      },
    }),
  );
  await readOnly.goto(url);
  await readOnly.getByRole("radio", { name: "Quality", exact: true }).waitFor();
  await readOnly.getByRole("radio", { name: "Quality", exact: true }).check();
  await readOnly.evaluate(() => window.remount());
  assert.equal(
    await readOnly
      .getByRole("radio", { name: "Quality", exact: true })
      .isChecked(),
    true,
    "rejected storage writes do not revert the mode when views remount",
  );
  await readOnly.close();
  console.log(
    "Display modes passed: raster cards and bid models, active-only collection playback, hover/focus, quality restoration, animation-only previews, failure placeholders, cancellation, persistence, cross-tab updates, keyboard controls and disabled storage.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
