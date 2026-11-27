import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const cacheDir = await mkdtemp(join(tmpdir(), "gift-sheet-"));
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><style>.swipe-art{height:220px}.swipe-long{height:900px}.swipe-controls{padding:16px}</style></head><body><div id="root"></div><script type="module">
import React,{useState}from'react';import{createRoot}from'react-dom/client';import{Modal}from'/src/ui/modal.tsx';import'/src/style.css';
function App(){const[open,setOpen]=useState(false),[guide,setGuide]=useState(false);window.openPreview=()=>setOpen(true);window.hidePreview=()=>setOpen(false);window.swipeCloseCalls=window.swipeCloseCalls||0;
return React.createElement('main',{className:'workspace'},React.createElement('button',{onClick:()=>setOpen(true)},'Open preview'),open&&React.createElement(Modal,{className:'detail',label:'Gift details',closeLabel:'Close details',viewportClose:true,swipeToClose:true,close:()=>{window.swipeCloseCalls++;setOpen(false)}},React.createElement('div',{className:'detail-art swipe-art'},'Artwork'),React.createElement('div',{className:'detail-content'},React.createElement('div',{className:'swipe-controls'},React.createElement('button',{onClick:()=>setGuide(true)},'Calculation guide'),React.createElement('a',{href:'#gift'},'Gift link'),React.createElement('input',{'aria-label':'Test input'})),React.createElement('div',{className:'swipe-long'},'Details'),guide&&React.createElement(Modal,{className:'valuation-popup',heading:'Calculation',label:'Calculation',closeLabel:'Close calculation',close:()=>setGuide(false)},React.createElement('p',null,'Calculation details')))));}
createRoot(document.getElementById('root')).render(React.createElement(App));</script></body></html>`;
const server = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "sheet-swipe",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url !== "/__swipe") return next();
          res.setHeader("Content-Type", "text/html");
          res.setHeader(
            "Content-Security-Policy",
            "style-src-attr 'none'; style-src-elem 'self' 'unsafe-inline'",
          );
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
  for (const reducedMotion of ["reduce", "no-preference"]) {
    const page = await browser.newPage({
      viewport: { width: 390, height: 844 },
      hasTouch: true,
      isMobile: true,
      reducedMotion,
    });
    page.setDefaultTimeout(10_000);
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    const cdp = await page.context().newCDPSession(page);
    const send = (type, points = []) =>
      cdp.send("Input.dispatchTouchEvent", { type, touchPoints: points });
    const sheet = page.getByRole("dialog", {
      name: "Gift details",
      exact: true,
    });
    const open = async () => {
      await page
        .getByRole("button", { name: "Open preview", exact: true })
        .click();
      await sheet.waitFor();
      await page.waitForTimeout(200);
    };
    const point = async (selector = ".detail-art") => {
      const box = await page.locator(selector).boundingBox();
      return {
        x: Math.round(box.x + box.width / 2),
        y: Math.round(box.y + Math.min(50, box.height / 2)),
        id: 1,
      };
    };
    const move = async (start, dx, dy) => {
      for (let step = 1; step <= 8; step++) {
        await send("touchMove", [
          {
            ...start,
            x: start.x + (dx * step) / 8,
            y: start.y + (dy * step) / 8,
          },
        ]);
        await page.waitForTimeout(20);
      }
    };
    const swipe = async (dx, dy, selector) => {
      const p = await point(selector);
      await send("touchStart", [p]);
      await move(p, dx, dy);
      await send("touchEnd");
    };
    await page.goto(server.resolvedUrls.local[0] + "__swipe");
    await open();
    const closeBounds = await page
      .getByRole("button", { name: "Close details", exact: true })
      .boundingBox();
    const p = await point();
    await send("touchStart", [p]);
    await move(p, 0, 30);
    assert.equal(await sheet.getAttribute("data-dragging"), "true");
    const dragged = await sheet.evaluate((el) => ({
      bottom: getComputedStyle(el).bottom,
      close: el.querySelector(".modal-close").getBoundingClientRect().top,
    }));
    assert(
      parseFloat(dragged.bottom) < -20,
      "sheet follows the finger under CSP",
    );
    assert.equal(
      dragged.close,
      closeBounds.y,
      "viewport close button stays fixed during a pull",
    );
    await send("touchEnd");
    await page.waitForTimeout(180);
    assert.equal(await sheet.getAttribute("data-dragging"), null);
    assert.equal(
      await sheet.evaluate((el) => getComputedStyle(el).bottom),
      "0px",
      "short pull snaps back",
    );
    await swipe(100, 15);
    assert.equal(await sheet.count(), 1, "horizontal swipes do not dismiss");
    await swipe(0, -160);
    await page.waitForTimeout(200);
    assert(
      await sheet.evaluate((el) => el.scrollTop > 0),
      "upward gesture scrolls the sheet natively",
    );
    await swipe(0, 160, ".detail-content");
    assert.equal(
      await sheet.count(),
      1,
      "a gesture starting in scrolled content does not dismiss",
    );
    await sheet.evaluate((el) => {
      el.scrollTop = 0;
    });
    await page.waitForTimeout(200);
    await swipe(0, 180, ".swipe-controls button");
    assert.equal(
      await sheet.count(),
      1,
      "buttons keep their native touch behavior",
    );
    await sheet.evaluate((el) => {
      el.scrollTop = 0;
    });
    await page
      .getByRole("button", { name: "Calculation guide", exact: true })
      .click();
    const guide = page.getByRole("dialog", {
      name: "Calculation",
      exact: true,
    });
    await guide.waitFor();
    await swipe(0, 100, ".valuation-popup p");
    assert.equal(
      await sheet.getAttribute("data-dragging"),
      null,
      "nested dialogs cannot drag the parent sheet",
    );
    await page.keyboard.press("Escape");
    await guide.waitFor({ state: "detached" });
    await sheet.evaluate((el) => {
      el.scrollTop = 0;
    });
    await swipe(0, 180);
    await sheet.waitFor({ state: "detached" });
    assert.equal(
      await page.evaluate(() => window.swipeCloseCalls),
      1,
      "one swipe causes exactly one close",
    );
    assert.equal(
      await page
        .getByRole("button", { name: "Open preview", exact: true })
        .evaluate((el) => el === document.activeElement),
      true,
      "dismissal restores focus",
    );
    await open();
    const multi = await point();
    await send("touchStart", [multi]);
    await move(multi, 0, 60);
    await send("touchMove", [
      { ...multi, y: multi.y + 60 },
      { x: multi.x + 40, y: multi.y + 60, id: 2 },
    ]);
    await send("touchEnd");
    assert.equal(
      await sheet.getAttribute("data-dragging"),
      null,
      "multitouch cancels the pull",
    );
    await page.setViewportSize({ width: 1200, height: 844 });
    await swipe(0, 180);
    assert.equal(
      await sheet.count(),
      1,
      "desktop dialogs do not swipe-dismiss",
    );
    await page.setViewportSize({ width: 390, height: 844 });
    const hidden = await point();
    await send("touchStart", [hidden]);
    await move(hidden, 0, 60);
    await page.evaluate(() => {
      Object.defineProperty(document, "hidden", {
        configurable: true,
        value: true,
      });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await send("touchEnd");
    assert.equal(await sheet.getAttribute("data-dragging"), null);
    await page.evaluate(() => {
      Object.defineProperty(document, "hidden", {
        configurable: true,
        value: false,
      });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await page
      .getByRole("button", { name: "Close details", exact: true })
      .click();
    await sheet.waitFor({ state: "detached" });
    assert.equal(await page.evaluate(() => window.swipeCloseCalls), 2);
    await open();
    const interrupted = await point();
    await send("touchStart", [interrupted]);
    await move(interrupted, 0, 60);
    await page.evaluate(() => window.hidePreview());
    await sheet.waitFor({ state: "detached" });
    await send("touchEnd");
    assert.equal(
      await page.evaluate(() => window.swipeCloseCalls),
      2,
      "unmounting mid-pull cannot invoke a stale close callback",
    );
    await open();
    await swipe(0, 180);
    await sheet.waitFor({ state: "detached" });
    assert.equal(
      await page.evaluate(() => window.swipeCloseCalls),
      3,
      "reopening installs a fresh gesture lifecycle",
    );
    assert.deepEqual(errors, []);
    await page.close();
  }
  console.log(
    "Sheet swipe passed: native touch dismissal, scroll/control protection, nested dialogs, fixed close button, cancellation, desktop and reduced motion.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
