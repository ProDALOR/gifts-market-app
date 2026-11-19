// Actual HTTP caching, real native image decoding and React visibility changes.
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const requests = new Map(),
  ready = new Set();
const count = (name) => requests.get(name) || 0;
const svg =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M100 100L400 100L400 400z" fill="#fff"/></svg>';
const png = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==",
  "base64",
);
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body><div id="fixture" style="padding:25px;height:2400px"></div><script type="module">
import React from 'react';import{createRoot}from'react-dom/client';import{GiftArtwork}from'/src/previews/gift-artwork.tsx';
const root=createRoot(document.getElementById('fixture'));
window.mount=(name,fallback='/media/still.png')=>root.render(React.createElement(GiftArtwork,{media:{thumbnail_url:fallback,preview_url:'/media/'+name+'.svg'},size:'large',playback:'static',fallback:'G'}));
window.clear=()=>root.render(null);
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
            req.url.endsWith(".svg") ? "image/svg+xml" : "image/png",
          );
          if (req.url === "/media/missing.svg") {
            res.statusCode = 404;
            res.setHeader("Cache-Control", "no-store");
            res.end();
          } else if (
            (req.url === "/media/recover.png" ||
              req.url === "/media/cancel.png" ||
              req.url.endsWith(".svg")) &&
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
  assert.deepEqual(errors, []);
  console.log(
    "Media retry browser passed: long Retry-After, visible recovery, cached decode/remount, cooldown reentry, cancellation, source changes, independent fallback recovery and invalid-image fallback.",
  );
} finally {
  await browser?.close();
  await server.close();
}
