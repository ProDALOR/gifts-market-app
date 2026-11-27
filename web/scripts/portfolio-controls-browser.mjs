import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const cacheDir = await mkdtemp(join(tmpdir(), "portfolio-controls-"));
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module">
import React,{useState}from'react';import{createRoot}from'react-dom/client';
import{PortfolioRefreshButton}from'/src/portfolio/refresh-button.tsx';import{AttributeArt}from'/src/orderbook/attribute-art.tsx';
import'/src/style.css';import'/src/portfolio/portfolio.css';
function App(){const[state,setState]=useState({ready:true,busy:false});window.setControls=patch=>setState(old=>({...old,...patch}));window.refreshCalls=window.refreshCalls||0;
return React.createElement('main',{style:{padding:16}},React.createElement(PortfolioRefreshButton,{...state,refresh:()=>window.refreshCalls++}),React.createElement('span',{className:'attribute-visual attribute-pattern'},React.createElement(AttributeArt,{option:{kind:'pattern',id:'test',name:'Test symbol',media:{thumbnail_url:'/media/patterns/test/thumbnail'}}})));}
createRoot(document.getElementById('root')).render(React.createElement(App));</script></body></html>`;
const server = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "portfolio-controls",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url !== "/__controls") return next();
          res.setHeader("Content-Type", "text/html");
          res.end(await server.transformIndexHtml(req.url, html));
        });
      },
    },
  ],
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  for (const width of [320, 1440]) {
    const page = await browser.newPage({
      viewport: { width, height: 800 },
      reducedMotion: "reduce",
    });
    page.setDefaultTimeout(10_000);
    const errors = [];
    let symbols = 0;
    page.on("pageerror", (error) => errors.push(error.message));
    await page.route("**/media/patterns/test/thumbnail", (route) => {
      symbols++;
      return route.fulfill({
        contentType: "image/svg+xml",
        body: '<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96"><path fill="black" d="M48 8 90 80H6Z"/></svg>',
      });
    });
    const epoch = new Date("2026-11-27T12:00:00Z");
    await page.clock.install({ time: epoch });
    await page.clock.pauseAt(new Date(epoch.getTime() + 1000));
    await page.goto(server.resolvedUrls.local[0] + "__controls");
    const button = page.getByRole("button", {
      name: "Refresh gifts",
      exact: true,
    });
    await button.waitFor();
    assert.equal(await button.isEnabled(), true);
    await button.click();
    assert.equal(await page.evaluate(() => window.refreshCalls), 1);
    await page.evaluate(() =>
      window.setControls({
        after: new Date(Date.now() + 65_000).toISOString(),
      }),
    );
    await page.getByText("1:05", { exact: true }).waitFor();
    assert.equal(await button.isDisabled(), true);
    assert.equal(
      await button.getAttribute("aria-description"),
      "Recheck available in 1:05",
    );
    const layout = await button.evaluate((button) => {
      const bounds = button.getBoundingClientRect(),
        text = button
          .querySelector(".portfolio-cooldown")
          .getBoundingClientRect();
      return {
        fits:
          text.left >= bounds.left &&
          text.right <= bounds.right &&
          text.top >= bounds.top &&
          text.bottom <= bounds.bottom,
        touch: bounds.width >= 44 && bounds.height >= 44,
        opacity: getComputedStyle(button).opacity,
      };
    });
    assert(layout.fits && layout.touch);
    assert.equal(layout.opacity, "1");
    // Hide/show within one rounded second: the previous timer could stop forever here.
    await page.clock.runFor(100);
    await page.evaluate(() => {
      Object.defineProperty(document, "hidden", {
        configurable: true,
        get: () => window.testHidden,
      });
      window.testHidden = true;
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await page.clock.runFor(100);
    await page.evaluate(() => {
      window.testHidden = false;
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await page.clock.runFor(1100);
    await page.getByText("1:04", { exact: true }).waitFor();
    await page.clock.runFor(64_000);
    await page.waitForFunction(
      () => !document.querySelector(".portfolio-refresh").disabled,
    );
    assert.equal(await page.locator(".portfolio-cooldown").count(), 0);
    await page.evaluate(() => window.setControls({ busy: true }));
    await page.waitForFunction(
      () =>
        document
          .querySelector(".portfolio-refresh")
          .getAttribute("aria-busy") === "true",
    );
    assert.equal(await button.isDisabled(), true);
    await page.evaluate(() =>
      window.setControls({ busy: false, ready: false }),
    );
    await page.waitForFunction(
      () =>
        document.querySelector(".portfolio-refresh").title ===
        "Waiting for connection",
    );
    assert.equal(await button.isDisabled(), true);
    await page.evaluate(() => window.setControls({ ready: true }));
    await page.waitForFunction(
      () => !document.querySelector(".portfolio-refresh").disabled,
    );

    await page.clock.resume();
    await page.locator(".attribute-pattern .gift-model-image.loaded").waitFor();
    for (const mode of ["dark", "light"]) {
      await page.evaluate(
        (mode) => (document.documentElement.dataset.colorScheme = mode),
        mode,
      );
      const appearance = await page
        .locator(".symbol-thumb")
        .evaluate((tile) => {
          const image = tile.querySelector("img"),
            style = getComputedStyle(image);
          return {
            background: getComputedStyle(tile).backgroundColor,
            surface: getComputedStyle(document.documentElement)
              .getPropertyValue("--surface")
              .trim(),
            filter: style.filter,
            loaded: image.naturalWidth > 0,
          };
        });
      const background = appearance.background
        .match(/[\d.]+/g)
        .slice(0, 3)
        .map(Number);
      const tint =
        Number(appearance.filter.match(/invert\(([^)]+)\)/)[1]) * 255;
      const luminance = (rgb) =>
        rgb
          .map((v) => v / 255)
          .map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4))
          .reduce((total, v, i) => total + v * [0.2126, 0.7152, 0.0722][i], 0);
      const a = luminance(background),
        b = luminance([tint, tint, tint]);
      assert(
        (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05) >= 4.5,
        `${mode} symbol has readable contrast`,
      );
      assert(appearance.loaded);
    }
    assert.equal(
      symbols,
      1,
      "theme changes reuse the same static symbol image",
    );
    assert.equal(
      await page.locator(".attribute-pattern .model-animation").count(),
      0,
    );
    assert.deepEqual(errors, []);
    await page.close();
  }
  console.log(
    "Portfolio controls passed: countdown expiry and same-second visibility recovery, busy/disconnected states, touch targets and cached symbols with light/dark contrast.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
