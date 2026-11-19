// Real HTTP/native decoding: local placeholders, SVG-first stills and bundled branding.
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const svg =
  '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><path fill="#fff" d="M128 128H384V384H128Z"/></svg>';
const requests = [],
  pending = new Map();
const release = () => {
  for (const send of pending.values()) send();
  pending.clear();
};
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body>
<div id="fixture" style="margin:50px;height:2400px"></div><script type="module">
import React from 'react';
import {createRoot} from 'react-dom/client';
import {GiftArtwork} from '/src/previews/gift-artwork.tsx';
import {BrandMark} from '/src/brand/brand-mark.tsx';
const root=createRoot(document.getElementById('fixture'));
window.mount=(name='slow')=>root.render(React.createElement('div',{style:{display:'flex',gap:12}},
 ...['gift','gift','symbol'].map((kind,index)=>React.createElement(GiftArtwork,{key:index,
 media:{thumbnail_url:'/media/'+name+'/'+index+'/thumbnail',preview_url:'/media/'+name+'/'+index+'/preview'},
 fallback:'X',playback:'static',placeholderKind:kind}))));
window.brand=()=>root.render(React.createElement('div',null,React.createElement(BrandMark,{animated:true}),React.createElement(BrandMark)));
window.unavailableSymbol=()=>root.render(React.createElement(GiftArtwork,{placeholderKind:'symbol',playback:'static',fallback:''}));
window.clear=()=>root.render(null);
window.fixtureReady=true;
</script></body></html>`;
const server = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "placeholder-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url === "/__placeholder_test") {
            res.setHeader("Content-Type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
            return;
          }
          if (!req.url.startsWith("/media/")) return next();
          requests.push(req.url);
          res.setHeader("Content-Type", "image/svg+xml");
          res.setHeader("Cache-Control", "no-store");
          const send = () => {
            if (!res.destroyed) res.end(svg);
          };
          if (req.url.includes("/slow/")) pending.set(req.url, send);
          else send();
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
  page.setDefaultTimeout(10000);
  const errors = [],
    brandRequests = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => {
    if (
      request.resourceType() === "fetch" &&
      new URL(request.url()).pathname.endsWith(".tgs")
    )
      brandRequests.push(request.url());
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__placeholder_test`,
  );
  await page.waitForFunction(() => window.fixtureReady);
  await page.evaluate(() => window.mount());
  await page.waitForFunction(
    () => document.querySelectorAll(".gift-placeholder-shimmer").length === 3,
  );
  assert.equal(await page.locator(".gift-placeholder-gift").count(), 2);
  assert.equal(await page.locator(".gift-placeholder-symbol").count(), 1);
  const paths = await page
    .locator(".gift-loading-placeholder clipPath path")
    .evaluateAll((paths) => paths.map((path) => path.getAttribute("d")));
  assert.equal(paths[0], paths[1]);
  assert.notEqual(
    paths[0],
    paths[2],
    "Symbols have their own local silhouette",
  );
  const ids = await page
    .locator(".gift-loading-placeholder [id]")
    .evaluateAll((nodes) => nodes.map((node) => node.id));
  assert.equal(
    new Set(ids).size,
    ids.length,
    "Concurrent shimmers have independent clip/gradient IDs",
  );
  await page.waitForFunction(() =>
    [...document.querySelectorAll(".gift-model-image")].every((image) =>
      image.src.endsWith("/preview"),
    ),
  );
  await page.evaluate(() => window.scrollTo(0, 1400));
  await page.waitForFunction(
    () => !document.querySelector(".gift-placeholder-shimmer"),
  );
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.waitForFunction(
    () => document.querySelectorAll(".gift-placeholder-shimmer").length === 3,
  );
  await page.emulateMedia({ reducedMotion: "reduce" });
  assert.equal(
    await page
      .locator(".gift-placeholder-shimmer")
      .first()
      .evaluate((node) => getComputedStyle(node).animationName),
    "none",
  );
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.evaluate(() => {
    window.hidden = true;
    Object.defineProperty(document, "hidden", {
      configurable: true,
      get: () => window.hidden,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForFunction(
    () => !document.querySelector(".gift-placeholder-shimmer"),
  );
  await page.evaluate(() => {
    window.hidden = false;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForFunction(
    () => document.querySelectorAll(".gift-placeholder-shimmer").length === 3,
  );
  release();
  await page.waitForFunction(
    () => document.querySelectorAll("img.loaded").length === 3,
  );
  assert.equal(await page.locator(".gift-loading-placeholder").count(), 0);
  await page.evaluate(() => window.mount("fast"));
  await page.waitForFunction(
    () =>
      document.querySelectorAll("img.loaded").length === 3 &&
      [...document.querySelectorAll("img")].every((image) =>
        image.src.includes("/fast/"),
      ),
  );
  assert.equal(
    await page.locator(".model-animation").count(),
    0,
    "SVG stills never allocate animation players",
  );
  assert.ok(
    requests.every((url) => url.endsWith("/preview")),
    "No PNG thumbnails or outline endpoints are requested",
  );
  await page.evaluate(() => window.unavailableSymbol());
  await page.locator(".gift-placeholder-symbol").waitFor();
  assert.equal(
    await page.locator(".gift-placeholder-shimmer").count(),
    0,
    "A missing symbol keeps its local shape without an endless loading animation",
  );
  await page.evaluate(() => window.clear());
  await page.waitForFunction(() => !document.querySelector(".gift-preview"));

  await page.evaluate(() => window.brand());
  const mark = page.locator(".brand").first();
  await mark.locator(".model-animation.ready").waitFor();
  assert.equal(
    await page.getByRole("link", { name: "IGIFTS home", exact: true }).count(),
    2,
  );
  assert.equal(
    await page.locator(".brand").last().locator(".model-animation").count(),
    0,
    "The footer uses a static SVG instead of a second player",
  );
  await mark.locator("img.loaded").waitFor();
  assert.equal(
    await mark
      .locator("img")
      .evaluate((image) => new URL(image.src).pathname.endsWith(".svg")),
    true,
  );
  assert.equal(brandRequests.length, 1);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.waitForFunction(
    () =>
      !document.querySelector(".model-animation canvas,.model-animation > svg"),
  );
  assert.equal(
    await mark
      .locator("img")
      .evaluate((image) => getComputedStyle(image).opacity),
    "1",
  );
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await mark.locator(".model-animation.ready").waitFor();
  await page.evaluate(() => window.scrollTo(0, 1400));
  await page.waitForFunction(
    () =>
      !document.querySelector(".model-animation canvas,.model-animation > svg"),
  );
  await page.evaluate(() => window.scrollTo(0, 0));
  await mark.locator(".model-animation.ready").waitFor();
  assert.equal(
    brandRequests.length,
    1,
    "Brand playback reuses the bundled, decoded animation",
  );
  await page.evaluate(() => window.clear());
  await page.waitForFunction(() => !document.querySelector(".gift-preview"));
  assert.deepEqual(errors, []);
  console.log(
    "Placeholder browser passed: zero outline/thumbnail requests, SVG-first stills, local gift/symbol shapes, unique shimmer IDs, visibility/motion cleanup and cached Liberty Figure branding.",
  );
} finally {
  release();
  await browser?.close();
  await server.close();
}
