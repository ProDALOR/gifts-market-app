// Preview geometry and canvas regression without hub/Telegram dependencies.
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body><div id="root"></div>
<script type="module">
import React from 'react';import {createRoot} from 'react-dom/client';
import {TraitPreview} from '/src/orderbook-ui.tsx';import {optionPredicate} from '/src/orderbook.ts';
const make=(id,name,attribute_id,meta={})=>({kind:'attribute_option',id,name,attribute_id,meta,aliases:[]});
const models=Array.from({length:30},(_,i)=>make('model-'+i,'Model '+String(i+1).padStart(2,'0'),'tg.model'));
const backdrops=Array.from({length:24},(_,i)=>make('backdrop-'+i,'Backdrop '+i,'tg.backdrop',{'colors.center_color':'#'+(0x906090+i*0x030502).toString(16),'colors.edge_color':'#'+(0x503850+i*0x030201).toString(16),'colors.pattern_color':'#'+(0x301830+i*0x010203).toString(16)}));
const patterns=Array.from({length:24},(_,i)=>make('symbol-'+i,'Symbol '+i,'tg.pattern'));
const options=new Map([...models,...backdrops,...patterns].map(o=>[o.id,o]));
const collection={id:'gift',attributes:Object.fromEntries([models,backdrops,patterns].map(rows=>[rows[0].attribute_id,{allowed_values:rows.map(o=>o.id)}]))};
const root=createRoot(document.getElementById('root'));
window.mount=({modelCount=0,backdropCount=0,patternCount=0,long=false}={})=>{
 const filters=Object.fromEntries([[models,modelCount],[backdrops,backdropCount],[patterns,patternCount]].filter(([,count])=>count>0).map(([rows,count])=>[rows[0].attribute_id,optionPredicate(rows.slice(0,count).map(o=>o.id))]));
 root.render(React.createElement('div',{className:'workspace'},React.createElement('main',{className:'orderbook'},React.createElement(TraitPreview,{collection,options,filters,reset:()=>window.mount()}),long?React.createElement('div',{id:'long-content',style:{height:1400}},'Long market'):null,React.createElement('footer',{className:'app-footer'},'IGIFTS'))));
};window.mount();
</script></body></html>`;
const server = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "trait-preview-fixture",
      configureServer(server) {
        server.middlewares.use(
          "/__trait_preview_test",
          async (req, res, next) => {
            try {
              res.setHeader("Content-Type", "text/html");
              res.end(
                await server.transformIndexHtml("/__trait_preview_test", html),
              );
            } catch (error) {
              next(error);
            }
          },
        );
      },
    },
  ],
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({
    viewport: { width: 390, height: 844 },
    reducedMotion: "reduce",
  });
  page.setDefaultTimeout(10000);
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const png = await page.evaluate(() => {
    const c = document.createElement("canvas");
    c.width = c.height = 16;
    const ctx = c.getContext("2d");
    ctx.fillStyle = "white";
    ctx.fillRect(3, 2, 10, 12);
    return c.toDataURL().split(",")[1];
  });
  await page.route("**/media/**", (route) =>
    route.fulfill({
      contentType: "image/png",
      body: Buffer.from(png, "base64"),
    }),
  );
  await page.addInitScript(() => {
    window.__patternDraws = [];
    window.__tints = new Set();
    const sources = new WeakMap();
    let next = 0;
    const draw = CanvasRenderingContext2D.prototype.drawImage,
      fill = CanvasRenderingContext2D.prototype.fillRect;
    CanvasRenderingContext2D.prototype.drawImage = function (source, ...args) {
      if (this.canvas.closest(".preview-symbols")) {
        if (!sources.has(source)) sources.set(source, ++next);
        window.__patternDraws.push([...args, sources.get(source)]);
      }
      return draw.call(this, source, ...args);
    };
    CanvasRenderingContext2D.prototype.fillRect = function (...args) {
      if (this.globalCompositeOperation === "source-in")
        window.__tints.add(this.fillStyle);
      return fill.apply(this, args);
    };
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__trait_preview_test`,
  );
  await page.waitForFunction(() => typeof window.mount === "function");
  assert.equal(await page.locator(".trait-preview").count(), 0);
  const bottom = () =>
    page
      .locator(".app-footer")
      .evaluate(
        (el) =>
          Math.abs(
            el.getBoundingClientRect().bottom -
              (innerHeight -
                parseFloat(getComputedStyle(el.parentElement).paddingBottom)),
          ) < 2,
      );
  assert.equal(
    await bottom(),
    true,
    "Empty pages anchor the footer above the bottom safe area",
  );
  await page.evaluate(() => window.mount({ modelCount: 1 }));
  await page.locator(".preview-model").waitFor();
  assert.equal(
    await page.locator(".preview-band").count(),
    0,
    "Unselected backdrops are not invented",
  );
  assert.equal(
    await page.locator(".preview-symbols").count(),
    0,
    "Unselected symbols are not invented",
  );
  await page.evaluate(() =>
    window.mount({ modelCount: 1, backdropCount: 12, patternCount: 12 }),
  );
  await page.waitForFunction(
    () => new Set(window.__patternDraws.map((draw) => draw[4])).size === 12,
  );
  assert.deepEqual(
    await page
      .locator(".preview-backdrop-row")
      .evaluateAll((rows) => rows.map((row) => row.children.length)),
    [6, 6],
  );
  assert.equal(
    await page.evaluate(() => window.__tints.size),
    1,
    "Every backdrop shares one pattern tint",
  );
  const prefix = () =>
    page.evaluate(() =>
      [
        ...new Map(
          window.__patternDraws
            .filter((draw) => draw[1] < 84)
            .map((draw) => [draw.slice(0, 2).join(","), draw]),
        ).values(),
      ].sort((a, b) => a[1] - b[1] || a[0] - b[0]),
    );
  const short = await prefix();
  await page.evaluate(() => {
    window.__patternDraws = [];
    window.mount({ modelCount: 20, backdropCount: 12, patternCount: 12 });
  });
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".preview-model").length === 20 &&
      window.__patternDraws.length > 80,
  );
  assert.deepEqual(
    await prefix(),
    short,
    "Growing model rows keeps the existing wallpaper phase and scale",
  );
  assert.ok(
    await page.evaluate(() =>
      window.__patternDraws.every((draw) => draw[2] === 14 && draw[3] === 14),
    ),
  );
  assert.deepEqual(
    await page
      .locator(".preview-model-row")
      .evaluateAll((rows) => rows.map((row) => row.children.length)),
    [5, 5, 5, 5],
  );
  const modelSize = () =>
    page
      .locator(".preview-model .gift-preview")
      .first()
      .evaluate((el) => el.getBoundingClientRect().width);
  assert.equal(await modelSize(), 40, "Five normal rows compact into four");
  await page.setViewportSize({ width: 480, height: 844 });
  await page.waitForFunction(
    () =>
      document
        .querySelector(".preview-model .gift-preview")
        .getBoundingClientRect().width === 52,
  );
  assert.deepEqual(
    await page
      .locator(".preview-model-row")
      .evaluateAll((rows) => rows.map((row) => row.children.length)),
    [5, 5, 5, 5],
  );
  assert.equal(
    await page.locator(".trait-preview").evaluate((el) => {
      const reset = el.querySelector(".reset-traits").getBoundingClientRect();
      return [...el.querySelectorAll(".preview-model > span")].every(
        (label) => label.getBoundingClientRect().bottom <= reset.top,
      );
    }),
    true,
    "Model labels clear the reset control",
  );
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => window.mount({ modelCount: 30 }));
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".preview-model").length === 30 &&
      document
        .querySelector(".preview-model .gift-preview")
        .getBoundingClientRect().width === 32,
  );
  assert.deepEqual(
    await page
      .locator(".preview-model-row")
      .evaluateAll((rows) => rows.map((row) => row.children.length)),
    [6, 6, 6, 6, 6],
  );
  for (const width of [320, 390, 480]) {
    await page.setViewportSize({ width, height: 844 });
    await page.waitForFunction(() => {
      const preview = document.querySelector(".trait-preview");
      const styles = getComputedStyle(preview);
      return parseFloat(styles.getPropertyValue("--preview-model-size")) === 32;
    });
    assert.equal(
      await page.locator(".trait-preview").evaluate((el) => {
        const reset = el.querySelector(".reset-traits").getBoundingClientRect();
        return (
          [...el.querySelectorAll(".preview-model")].every((model) => {
            const image = model
                .querySelector(".gift-preview")
                .getBoundingClientRect(),
              label = model
                .querySelector("span[title]")
                .getBoundingClientRect(),
              bounds = model.getBoundingClientRect();
            return image.width <= bounds.width && label.bottom <= reset.top;
          }) && document.documentElement.scrollWidth <= innerWidth
        );
      }),
      true,
      `Compact models fit their cells and clear reset at ${width}px`,
    );
  }
  await page.evaluate(() => window.mount({ modelCount: 1 }));
  await page.waitForFunction(
    () => document.querySelectorAll(".preview-model").length === 1,
  );
  assert.equal(
    await modelSize(),
    52,
    "Removing models restores the normal size",
  );
  await page.evaluate(() =>
    window.mount({ backdropCount: 13, patternCount: 12 }),
  );
  await page.waitForFunction(
    () => document.querySelectorAll(".preview-band").length === 13,
  );
  assert.deepEqual(
    await page
      .locator(".preview-backdrop-row")
      .evaluateAll((rows) => rows.map((row) => row.children.length)),
    [7, 6],
  );
  assert.equal(await page.locator(".preview-model").count(), 0);
  await page.getByRole("button", { name: "Reset traits", exact: true }).click();
  assert.equal(await page.locator(".trait-preview").count(), 0);
  assert.equal(await bottom(), true);
  await page.evaluate(() => window.mount({ long: true }));
  assert.equal(
    await page
      .locator(".app-footer")
      .evaluate(
        (el) =>
          el.getBoundingClientRect().top >=
          document.getElementById("long-content").getBoundingClientRect()
            .bottom,
      ),
    true,
    "Footer follows long content without covering it",
  );
  for (const width of [320, 390, 480]) {
    await page.setViewportSize({ width, height: 844 });
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
      false,
    );
  }
  assert.deepEqual(errors, []);
  console.log(
    "Trait previews passed: balanced backdrop rows, fixed monochrome wallpaper, adaptive model sizing, hidden unselected preview and short/long footer placement.",
  );
} finally {
  await browser?.close();
  await server.close();
}
