// Real HTTP (including Vary: Origin), React and Lottie. No route interception,
// so an accidental CSS-image load would be counted as a second request.
import assert from "node:assert/strict";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { createServer } from "vite";
import { chromium } from "playwright";

const svg =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path fill="#ffffff" d="M128 128L384 128L384 384L128 384z"/></svg>';
const png = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==",
  "base64",
);
const tgs = gzipSync(
  JSON.stringify({
    v: "5.7.4",
    w: 512,
    h: 512,
    fr: 30,
    ip: 0,
    op: 60,
    assets: [],
    layers: [
      {
        ty: 4,
        ind: 1,
        ip: 0,
        op: 60,
        st: 0,
        ks: {
          o: { a: 0, k: 100 },
          r: { a: 0, k: 0 },
          p: { a: 0, k: [0, 0] },
          a: { a: 0, k: [0, 0] },
          s: { a: 0, k: [100, 100] },
        },
        shapes: [
          {
            ty: "rc",
            p: { a: 0, k: [256, 256] },
            s: { a: 0, k: [128, 128] },
            r: { a: 0, k: 0 },
          },
          { ty: "fl", c: { a: 0, k: [1, 0, 0, 1] }, o: { a: 0, k: 100 }, r: 1 },
        ],
      },
    ],
  }),
);
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body>
<div id="fixture" style="margin:50px;height:2400px"></div><script type="module">
import React from 'react';
import {createRoot} from 'react-dom/client';
import {GiftArtwork} from '/src/previews/gift-artwork.tsx';
import {decodeOutline} from '/src/previews/outline-resource.ts';
const root=createRoot(document.getElementById('fixture'));
window.mount=(name,motion=false,copies=1,outline=name,thumbnail=true)=>root.render(React.createElement('div',null,
 ...Array.from({length:copies},(_,i)=>React.createElement(GiftArtwork,{key:i,
 media:{thumbnail_url:thumbnail?'/media/models/'+name+'/thumbnail':undefined,placeholder_url:'/media/models/'+outline+'/placeholder',animation_url:motion?'/media/models/'+name+'/animation':undefined},
 fallback:'G',playback:motion?'detail':'static'}))));
window.clear=()=>root.render(null);
window.decode=(text)=>decodeOutline(new TextEncoder().encode(text));
// Control only outline debounce timers to test a ready animation deterministically.
const timeout=window.setTimeout,clear=window.clearTimeout,held=new Map();let next=-1;
window.holdOutlines=false;window.heldCount=()=>held.size;
window.setTimeout=(fn,ms,...args)=>{if(window.holdOutlines&&ms===150){const id=next--;held.set(id,()=>fn(...args));return id;}return timeout(fn,ms,...args);};
window.clearTimeout=(id)=>{held.delete(id);clear(id);};
window.releaseOutlines=()=>{window.holdOutlines=false;const callbacks=[...held.values()];held.clear();callbacks.forEach(fn=>fn());};
window.fixtureReady=true;
</script></body></html>`;
const requests = [],
  aborted = [],
  pending = new Map();
const count = (name, variant = "placeholder") =>
  requests.filter((url) => url === `/media/models/${name}/${variant}`).length;
const release = (name, variant) =>
  pending.get(`/media/models/${name}/${variant}`)?.forEach((send) => send());
async function until(condition) {
  for (let i = 0; i < 500; i++) {
    if (condition()) return;
    await delay(10);
  }
  assert.fail("Timed out waiting for fixture request/cancellation");
}
const server = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "outline-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url === "/__outline_test") {
            res.setHeader("Content-Type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
            return;
          }
          if (!req.url.startsWith("/media/")) return next();
          requests.push(req.url);
          res.on("close", () => {
            if (!res.writableEnded) aborted.push(req.url);
          });
          const [, , , name, variant] = req.url.split("/");
          res.setHeader("Vary", "Origin, Accept-Encoding");
          // Disable HTTP caching: assertions below must be satisfied by our shared resource.
          res.setHeader("Cache-Control", "no-store");
          res.setHeader(
            "Content-Type",
            variant === "animation"
              ? "application/x-tgsticker"
              : variant === "placeholder"
                ? "image/svg+xml"
                : "image/png",
          );
          const send = () => {
            if (res.destroyed || res.writableEnded) return;
            if (name === "missing" && variant === "placeholder") {
              res.statusCode = 404;
              res.end();
            } else if (name === "failed-thumbnail" && variant === "thumbnail") {
              res.statusCode = 404;
              res.end();
            } else
              res.end(
                variant === "animation"
                  ? tgs
                  : variant === "placeholder"
                    ? svg
                    : png,
              );
          };
          if (
            (variant === "thumbnail" &&
              !["fast", "failed-thumbnail"].includes(name)) ||
            (variant === "animation" &&
              ["during", "failed-thumbnail", "animation-only"].includes(
                name,
              )) ||
            (variant === "placeholder" &&
              ["cancel", "during", "still-ready", "changed"].includes(name))
          ) {
            if (!pending.has(req.url)) pending.set(req.url, []);
            pending.get(req.url).push(send);
          } else send();
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
    `http://127.0.0.1:${server.httpServer.address().port}/__outline_test`,
  );
  await page.waitForFunction(() => window.fixtureReady);

  await page.evaluate(() => {
    window.holdOutlines = true;
    window.mount("shared", false, 2);
  });
  await page.waitForFunction(
    () => document.querySelectorAll(".gift-loading-placeholder").length === 2,
  );
  assert.equal(
    await page.locator(".gift-preview-fallback").count(),
    0,
    "Loading never flashes an initial letter",
  );
  assert.equal(
    count("shared"),
    0,
    "The local silhouette needs no network request",
  );
  await page.evaluate(() => window.releaseOutlines());
  await page.waitForFunction(
    () => document.querySelectorAll(".gift-model-placeholder").length === 2,
  );
  assert.equal(
    count("shared"),
    1,
    "Two copies share one HTTP fetch, without CSS mask reloads",
  );
  const ids = await page
    .locator(".gift-model-placeholder clipPath")
    .evaluateAll((nodes) => nodes.map((node) => node.id));
  assert.equal(
    new Set(ids).size,
    2,
    "Each rendered outline has its own local clip ID",
  );
  await page.evaluate(() => window.clear());
  await page.waitForFunction(() => !document.querySelector(".gift-preview"));
  await page.evaluate(() => {
    window.holdOutlines = true;
    window.mount("shared", false, 2);
  });
  await page.waitForFunction(
    () => document.querySelectorAll(".gift-model-placeholder").length === 2,
  );
  assert.equal(
    await page.evaluate(() => window.heldCount()),
    0,
    "Warm outlines skip the cold debounce after remounts",
  );
  assert.equal(count("shared"), 1);
  await page.evaluate(() => window.releaseOutlines());
  assert.equal(
    await page
      .locator(".gift-placeholder-shimmer")
      .first()
      .evaluate((node) => getComputedStyle(node).animationName),
    "gift-placeholder-shimmer",
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
  await page.evaluate(() => window.scrollTo(0, 1300));
  await page.waitForFunction(
    () => !document.querySelector(".gift-model-placeholder"),
  );
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.waitForFunction(
    () => document.querySelectorAll(".gift-model-placeholder").length === 2,
  );
  assert.equal(count("shared"), 1, "Reentry reuses decoded outlines");

  await page.evaluate(() => {
    window.poster = document.querySelector("img.gift-model-image");
    window.mount("shared", false, 2, "changed");
  });
  await until(() => count("changed") === 1);
  assert.equal(
    await page.locator(".gift-model-placeholder").count(),
    0,
    "Source changes never display the old path",
  );
  assert.equal(
    await page.evaluate(
      () => window.poster === document.querySelector("img.gift-model-image"),
    ),
    true,
    "An outline change preserves the artwork/player identity",
  );
  release("changed", "placeholder");
  await page.waitForFunction(
    () => document.querySelectorAll(".gift-model-placeholder").length === 2,
  );

  await page.evaluate(() => window.mount("cancel"));
  await until(() => count("cancel") === 1);
  await page.evaluate(() => window.mount("replacement"));
  release("shared", "thumbnail");
  release("cancel", "thumbnail");
  await page.locator(".gift-model-placeholder").waitFor();
  await until(() => aborted.includes("/media/models/cancel/placeholder"));
  release("cancel", "placeholder");
  assert.equal(count("replacement"), 1);

  for (let i = 0; i < 2; i++) {
    await page.evaluate(
      (i) => window.mount(`missing-${i}`, false, 1, "missing"),
      i,
    );
    await page.waitForTimeout(250);
    assert.equal(await page.locator(".gift-model-placeholder").count(), 0);
    assert.equal(
      await page.locator(".gift-loading-placeholder").count(),
      1,
      "Missing outlines retain a local silhouette",
    );
    assert.equal(await page.locator(".gift-preview-fallback").count(), 0);
    await page.evaluate(() => window.clear());
    await page.waitForFunction(() => !document.querySelector(".gift-preview"));
    release(`missing-${i}`, "thumbnail");
  }
  assert.equal(count("missing"), 1, "404 is negatively cached across remounts");
  release("replacement", "thumbnail");
  await page.evaluate(() => window.mount("fast"));
  await page.locator("img.loaded").waitFor();
  await page.waitForTimeout(180);
  assert.equal(count("fast"), 0, "Fast artwork cancels the debounce");

  await page.evaluate(() => {
    window.holdOutlines = true;
    window.mount("ready", true);
  });
  await page.locator(".model-animation.ready").waitFor();
  assert.equal(
    await page.evaluate(() => window.heldCount()),
    0,
    "Ready animation cancels the outline timer",
  );
  await page.evaluate(() => window.releaseOutlines());
  assert.equal(count("ready"), 0);
  assert.equal(
    await page
      .locator(".gift-preview-fallback,.gift-model-placeholder")
      .count(),
    0,
    "Animation readiness removes the loading state even with a pending still",
  );
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.locator(".gift-model-placeholder").waitFor();
  assert.equal(
    await page.locator(".model-animation.ready").count(),
    0,
    "Stopping animation restores loading while the still is pending",
  );
  await page.emulateMedia({ reducedMotion: "no-preference" });

  await page.evaluate(() => window.mount("during", true));
  release("ready", "thumbnail");
  await until(
    () => count("during") === 1 && count("during", "animation") === 1,
  );
  release("during", "animation");
  await page.locator(".model-animation.ready").waitFor();
  await until(() => aborted.includes("/media/models/during/placeholder"));
  release("during", "placeholder");
  assert.equal(
    await page.locator(".gift-model-placeholder").count(),
    0,
    "Late outline completion cannot restore a shimmer over animation",
  );

  await page.evaluate(() => window.mount("still-ready"));
  await until(() => count("still-ready") === 1);
  release("still-ready", "thumbnail");
  await page.locator("img.loaded").waitFor();
  await until(() => aborted.includes("/media/models/still-ready/placeholder"));
  assert.equal(await page.locator(".gift-model-placeholder").count(), 0);

  for (const name of ["failed-thumbnail", "animation-only"]) {
    await page.evaluate(
      (name) => window.mount(name, true, 1, name, name !== "animation-only"),
      name,
    );
    await page.locator(".gift-model-placeholder").waitFor();
    assert.equal(
      await page.locator(".gift-preview-fallback").count(),
      0,
      "A missing/failed poster cannot disable outlines",
    );
    if (name === "failed-thumbnail") {
      await page.waitForFunction(
        () => !document.querySelector("img.gift-model-image"),
      );
      assert.equal(
        await page.locator(".gift-placeholder-shimmer").count(),
        0,
        "Failed stills retain a static silhouette",
      );
    }
    release(name, "animation");
    await page.locator(".model-animation.ready").waitFor();
    assert.equal(
      await page.locator(".gift-loading-placeholder").count(),
      0,
      "Animation replaces the silhouette",
    );
  }

  assert.equal(
    await page.evaluate((source) => window.decode(source), svg),
    "M128 128L384 128L384 384L128 384z",
  );
  for (const invalid of [
    "<!DOCTYPE svg>" + svg,
    svg.replace("<path", '<path onclick="alert(1)"'),
    svg.replace("</svg>", "<script>alert(1)</script></svg>"),
    svg.replace("#ffffff", "url(https://example.com)"),
    svg.replace("M128", "bad128"),
    svg.replace("<path", '<path href="https://example.com"'),
    svg.replace("</svg>", "<image href='https://example.com'/></svg>"),
    " ".repeat(8193),
  ]) {
    assert.equal(
      await page.evaluate((source) => {
        try {
          window.decode(source);
          return false;
        } catch {
          return true;
        }
      }, invalid),
      true,
      "Only bounded passive path data is accepted",
    );
  }
  await page.evaluate(() => window.clear());
  assert.deepEqual(errors, []);
  console.log(
    "Outline browser passed: one fetch, shared paths, local SVG IDs, motion, scrolling, source changes, negative caching, ready still/animation cancellation and passive SVG validation.",
  );
} finally {
  await browser?.close();
  await server.close();
}
