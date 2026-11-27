import assert from "node:assert/strict";
import { mkdtemp, rm, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const cacheDir = await mkdtemp(join(tmpdir(), "rarity-picker-"));
const mediaReads = new Map();
const html = `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/src/style.css"></head><body><div id="fixture"></div><script type="module">
import React,{useState,useMemo}from'react';import{createRoot}from'react-dom/client';
import{TraitPickers}from'/src/collections/trait-pickers.tsx';
const uniqueMedia = new URLSearchParams(location.search).has('media');
const pct=permille=>({kind:'percentage',permille});
const media={thumbnail_url:'/art.svg',preview_url:'/art.svg'};
function App(){const[collection,select]=useState('a'),[open,setOpen]=useState(),[filters,setFilters]=useState({}),[revised,revise]=useState(false);
window.chooseCollection=id=>{select(id);setFilters({});};window.reviseRarity=()=>revise(true);window.currentFilters=filters;window.openPicker=setOpen;
const options=useMemo(()=>new Map((collection==='a'?[
{id:'amber',name:'Amber',kind:'model',rarity:pct(revised?10:5),media},
{id:'azure',name:'Azure',kind:'model',rarity:pct(5),media},
{id:'blue',name:'Blue',kind:'model',rarity:pct(10),media},
{id:'unknown',name:'Mystery',kind:'model',media},
{id:'legendary',name:'Craft legendary',kind:'model',rarity:{kind:'legendary'},media},
{id:'epic',name:'Craft epic',kind:'model',rarity:{kind:'epic'},media},
{id:'rare',name:'Craft rare',kind:'model',rarity:{kind:'rare'},media},
{id:'uncommon',name:'Craft uncommon',kind:'model',rarity:{kind:'uncommon'},media},
...Array.from({length:uniqueMedia?200:60},(_,i)=>({id:'many-'+i,name:'Long model name '+i,kind:'model',rarity:pct(20+i%2),media})),
{id:'red',name:'Red',kind:'backdrop',rarity:pct(5),palette:{center:'#664455',edge:'#332233',pattern:'#222222'}},
{id:'black',name:'Black',kind:'backdrop',rarity:pct(5),palette:{center:'#443344',edge:'#221122',pattern:'#111111'}},
{id:'star',name:'Star',kind:'pattern',rarity:{kind:'rare'},media},
{id:'moon',name:'Moon',kind:'pattern',rarity:pct(1),media},
]:[{id:'b-model',name:'Other collection model',kind:'model',rarity:pct(5),media}]).map(o=>[o.id,uniqueMedia&&o.media?{...o,media:{thumbnail_url:'/art.svg?id='+o.id+'&format=thumbnail',preview_url:'/art.svg?id='+o.id+'&format=preview',animation_url:'/unwanted.tgs?id='+o.id}}:o])),[collection,revised]);
return React.createElement('div',{className:'workspace'},React.createElement('main',{className:'orderbook market-shell'},React.createElement('aside',{className:'market-sidebar'},React.createElement(TraitPickers,{collection:{id:collection,name:collection},options,filters,open,setOpen,change:(key,value)=>setFilters({...filters,[key]:value})}),React.createElement('button',{id:'other-control',onClick:()=>window.otherClicks=(window.otherClicks??0)+1},'Other control')),React.createElement('section',{className:'market-results',style:{minHeight:1800}},'Listings')));}
const root=createRoot(document.getElementById('fixture'));root.render(React.createElement(App));window.unmount=()=>root.unmount();
</script></body></html>`;
const server = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "rarity-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url.split("?")[0] === "/__rarity") {
            res.setHeader("Content-Type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
            return;
          }
          if (req.url.startsWith("/art.svg")) {
            mediaReads.set(req.url, (mediaReads.get(req.url) || 0) + 1);
            res.setHeader(
              "Cache-Control",
              "public, max-age=31536000, immutable",
            );
            res.setHeader("Content-Type", "image/svg+xml");
            res.end(
              '<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><circle cx="50" cy="50" r="40" fill="#b18eb0"/></svg>',
            );
            return;
          }
          next();
        });
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
  await page.addInitScript(() => {
    window.pickerAnimations = [];
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (...args) {
      const animation = animate.apply(this, args);
      window.pickerAnimations.push(animation);
      return animation;
    };
    window.pickerResizeTargets = new Set();
    const Native = window.ResizeObserver;
    window.ResizeObserver = class extends Native {
      targets = new Set();
      observe(target, options) {
        if (target.matches(".picker-panel")) {
          this.targets.add(target);
          window.pickerResizeTargets.add(target);
        }
        super.observe(target, options);
      }
      unobserve(target) {
        this.targets.delete(target);
        window.pickerResizeTargets.delete(target);
        super.unobserve(target);
      }
      disconnect() {
        for (const target of this.targets)
          window.pickerResizeTargets.delete(target);
        this.targets.clear();
        super.disconnect();
      }
    };
  });
  const errors = [];
  const apiRequests = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => {
    if (
      /^\/(collections|catalog|valuations|ws)(\?|$|\/)/.test(
        new URL(request.url()).pathname,
      )
    )
      apiRequests.push(request.url());
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__rarity`,
  );
  const button = (name) => page.getByRole("button", { name, exact: true });
  const check = (name) =>
    page.getByRole("button", { name: `${name} rarity`, exact: true });
  const checked = async (name, value) => {
    await page.waitForFunction(
      ({ name, value }) =>
        [...document.querySelectorAll(".trait-group-toggle")].some(
          (button) =>
            button.getAttribute("aria-label") === `${name} rarity` &&
            button.getAttribute("aria-pressed") === value,
        ),
      { name, value: String(value) },
    );
    assert.equal(await check(name).getAttribute("aria-pressed"), String(value));
  };
  await button("Model").click();
  assert.deepEqual(
    await page
      .locator(".trait-group-toggle")
      .evaluateAll((rows) => rows.map((row) => row.getAttribute("aria-label"))),
    ["0.5% rarity", "1% rarity", "2% rarity", "2.1% rarity", "Unknown rarity"],
  );
  const search = page.getByRole("searchbox");
  assert.equal(await button("Primary").getAttribute("aria-pressed"), "true");
  assert.equal(await check("0.5%").locator(".trait-group-count").count(), 0);
  assert.equal(await check("1%").locator(".trait-group-count").count(), 0);
  assert.equal(
    await check("2%").locator(".trait-group-count").textContent(),
    "30",
  );
  await button("Amber").click();
  await page
    .locator(".trait-options")
    .evaluate((node) => (node.scrollTop = 80));
  await button("Craftable").click();
  assert.equal(
    await page.locator(".trait-options").evaluate((node) => node.scrollTop),
    0,
  );
  assert.deepEqual(
    await page.evaluate(() => window.currentFilters.models),
    ["amber"],
    "Switching groups does not select models",
  );
  assert.deepEqual(await page.locator(".trait-group-label").allTextContents(), [
    "Legendary",
    "Epic",
    "Rare",
    "Uncommon",
  ]);
  const tierColors = await page
    .locator(".trait-group-toggle")
    .evaluateAll((buttons) =>
      buttons.map((button) => getComputedStyle(button).color),
    );
  assert.equal(
    new Set(tierColors).size,
    4,
    "Each craft tier has its own color",
  );
  await button("Craft rare").click();
  await search.fill("legendary");
  assert.equal(await page.locator(".trait-group").count(), 1);
  await button("Primary").click();
  await page.getByText("No matching traits", { exact: true }).waitFor();
  assert.deepEqual(
    await page.evaluate(() => window.currentFilters.models),
    ["amber", "rare"],
    "Selections from both groups survive switching and search",
  );
  await search.fill("");
  assert.equal(await button("Amber").getAttribute("aria-pressed"), "true");
  await button("Craftable").click();
  await checked("Rare", true);
  await mkdir("/tmp/gifts-market-ui", { recursive: true });
  for (const width of [320, 1200]) {
    await page.setViewportSize({ width, height: 900 });
    await page.screenshot({
      path: `/tmp/gifts-market-ui/craft-picker-${width}.png`,
    });
    await page.evaluate(() => {
      document.documentElement.classList.add("telegram");
      document.documentElement.dataset.colorScheme = "light";
    });
    const lightColors = await page
      .locator(".trait-group-toggle")
      .evaluateAll((buttons) =>
        buttons.map((button) => getComputedStyle(button).color),
      );
    assert.equal(new Set(lightColors).size, 4);
    assert.notDeepEqual(
      lightColors,
      tierColors,
      "Craft colors adapt to light theme",
    );
    await page.evaluate(() => {
      document.documentElement.classList.remove("telegram");
      delete document.documentElement.dataset.colorScheme;
    });
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await button("Primary").click();
  await button("Any").click();
  await check("0.5%").click();
  await checked("0.5%", true);
  assert.deepEqual(await page.evaluate(() => window.currentFilters.models), [
    "amber",
    "azure",
  ]);
  await button("Amber").click();
  await checked("0.5%", "mixed");
  await check("0.5%").focus();
  await page.keyboard.press("Space");
  await checked("0.5%", true);
  await check("1%").click();
  await search.fill("  Amber ");
  assert.equal(await page.locator(".trait-group").count(), 1);
  await check("0.5%").click();
  assert.deepEqual(await page.evaluate(() => window.currentFilters.models), [
    "azure",
    "blue",
  ]);
  await search.fill("unfindable");
  assert.equal(
    await page.getByText("No matching traits", { exact: true }).count(),
    1,
  );
  await search.fill("");
  await checked("0.5%", "mixed");
  await button("Any").click();
  assert.equal(await button("Any").getAttribute("aria-pressed"), "true");
  await button("Craftable").click();
  await check("Rare").click();
  await button("Primary").click();
  assert.deepEqual(await page.evaluate(() => window.currentFilters.models), [
    "rare",
  ]);
  await check("Unknown").click();
  assert.deepEqual(await page.evaluate(() => window.currentFilters.models), [
    "rare",
    "unknown",
  ]);
  await button("Backdrop").click();
  assert.equal(await page.locator(".model-type-options").count(), 0);
  await check("0.5%").click();
  assert.deepEqual(await page.evaluate(() => window.currentFilters.backdrops), [
    "black",
    "red",
  ]);
  await button("Symbol").click();
  await check("Rare").click();
  assert.deepEqual(await page.evaluate(() => window.currentFilters.patterns), [
    "star",
  ]);
  await button("Model").click();
  await button("Any").click();
  await button("Amber").click();
  await page.evaluate(() => window.reviseRarity());
  await checked("1%", "mixed");
  await checked("0.5%", false);
  assert.equal(
    await page
      .getByRole("group", { name: "1% rarity", exact: true })
      .getByRole("button", { name: "Amber", exact: true })
      .count(),
    1,
  );
  assert.deepEqual(await page.evaluate(() => window.currentFilters.models), [
    "amber",
  ]);
  await mkdir("/tmp/gifts-market-ui", { recursive: true });
  for (const width of [320, 390, 1200]) {
    await page.setViewportSize({ width, height: 900 });
    const layout = await page.locator(".trait-options").evaluate((panel) => ({
      bounded: panel.clientHeight <= 220,
      scrolls: panel.scrollHeight > panel.clientHeight,
      overflow: document.documentElement.scrollWidth > innerWidth,
      groups: [...panel.querySelectorAll(".trait-group-toggle")].every(
        (button) => {
          const a = button.getBoundingClientRect(),
            b = panel.getBoundingClientRect(),
            choices = button.nextElementSibling.getBoundingClientRect(),
            label = button.querySelector(".trait-group-label");
          return (
            a.left >= b.left &&
            a.right <= choices.left &&
            choices.right <= b.right + 1 &&
            getComputedStyle(label).writingMode === "vertical-rl" &&
            a.height >= label.getBoundingClientRect().height
          );
        },
      ),
      fullWidthAny:
        Math.abs(
          panel.querySelector(":scope > .trait-option").getBoundingClientRect()
            .width - panel.clientWidth,
        ) <= 1,
    }));
    assert.deepEqual(layout, {
      bounded: true,
      scrolls: true,
      overflow: false,
      groups: true,
      fullWidthAny: true,
    });
    await page.screenshot({
      path: `/tmp/gifts-market-ui/rarity-picker-${width}.png`,
    });
  }
  await page.evaluate(() => window.chooseCollection("b"));
  await button("Other collection model").waitFor();
  assert.equal(await page.locator(".model-type-options").count(), 0);
  await checked("0.5%", false);
  assert.equal(await button("Amber").count(), 0);
  await check("0.5%").click();
  assert.deepEqual(await page.evaluate(() => window.currentFilters.models), [
    "b-model",
  ]);
  await page.evaluate(() => {
    window.openPicker(undefined);
    window.chooseCollection("a");
  });
  await page.locator(".trait-panel-shell").waitFor({ state: "detached" });
  assert.equal(
    await page.evaluate(
      () =>
        window.pickerAnimations.filter((a) => a.id.startsWith("trait-panel"))
          .length,
    ),
    0,
    "Reduced motion opens and closes without animations",
  );
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await button("Model").click();
  await page.waitForFunction(() =>
    window.pickerAnimations.some((a) => a.id === "trait-panel-visibility"),
  );
  const firstFrames = await page.evaluate(() =>
    window.pickerAnimations
      .find((a) => a.id === "trait-panel-visibility")
      .effect.getKeyframes(),
  );
  assert.equal(firstFrames[0].opacity, "0");
  assert.equal(firstFrames.at(-1).opacity, "1");
  assert(
    firstFrames.every(
      (frame) => frame.height === undefined && frame.width === undefined,
    ),
    "Picker animations never resize layout",
  );
  await page.locator(".trait-panel-shell").evaluate(async (shell) => {
    window.pickerShell = shell;
    await Promise.all(
      shell
        .getAnimations({ subtree: true })
        .map((a) => a.finished.catch(() => {})),
    );
  });
  await button("Backdrop").click();
  await page
    .getByRole("searchbox", { name: "Search backdrop choices" })
    .waitFor();
  assert.equal(
    await page.evaluate(
      () => document.querySelector(".trait-panel-shell") === window.pickerShell,
    ),
    true,
    "Switches reuse the same panel shell",
  );
  const animationsBeforeSearch = await page.evaluate(
    () => window.pickerAnimations.length,
  );
  await page.getByRole("searchbox").fill("No matching backdrop");
  await page.getByText("No matching traits", { exact: true }).waitFor();
  assert.equal(
    await page
      .getByRole("searchbox")
      .evaluate((input) => input === document.activeElement),
    true,
    "Search retains keyboard focus",
  );
  assert.equal(
    await page.evaluate(() => window.pickerAnimations.length),
    animationsBeforeSearch,
    "Typing does not restart picker fades",
  );
  await page.keyboard.press("Escape");
  await page.waitForFunction(
    () => document.querySelector(".trait-panel-shell")?.inert,
  );
  assert.equal(
    await page.evaluate(() => document.activeElement.id),
    "pick-backdrops",
  );
  assert.equal(
    await page.getByRole("searchbox").count(),
    0,
    "Closing content is inert and excluded from accessibility",
  );
  await page.locator(".trait-panel-shell").waitFor({ state: "detached" });
  await button("Model").click();
  await page.waitForFunction(
    () =>
      document.querySelector(".trait-panel-shell")?.getBoundingClientRect()
        .height > 40,
  );
  await page.evaluate(() => {
    window.openPicker(undefined);
  });
  await page.waitForFunction(
    () => document.querySelector(".trait-panel-shell")?.inert,
  );
  await button("Symbol").click();
  await page
    .getByRole("searchbox", { name: "Search symbol choices" })
    .waitFor();
  await page.locator(".trait-panel-shell").evaluate(async (shell) => {
    await Promise.all(
      shell
        .getAnimations({ subtree: true })
        .map((a) => a.finished.catch(() => {})),
    );
  });
  assert.equal(
    await page
      .getByRole("searchbox", { name: "Search symbol choices" })
      .count(),
    1,
    "A stale close callback cannot remove the reopened panel",
  );
  await page.getByRole("searchbox").focus();
  await page.keyboard.press("Escape");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.locator(".trait-panel-shell").waitFor({ state: "detached" });
  assert.equal(
    await page.evaluate(
      () =>
        window.pickerAnimations.filter(
          (a) => a.id.startsWith("trait-panel") && a.playState === "running",
        ).length,
    ),
    0,
    "Reduced motion cancels active transitions",
  );
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await button("Model").click();
  await page.waitForFunction(
    () =>
      document.querySelector(".trait-panel-shell")?.getBoundingClientRect()
        .height > 40,
  );
  await page.evaluate(() => {
    window.openPicker(undefined);
  });
  await page.waitForFunction(
    () => document.querySelector(".trait-panel-shell")?.inert,
  );
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", {
      configurable: true,
      value: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.locator(".trait-panel-shell").waitFor({ state: "detached" });
  assert.equal(
    await page.evaluate(
      () =>
        window.pickerAnimations.filter(
          (a) => a.id.startsWith("trait-panel") && a.playState === "running",
        ).length,
    ),
    0,
    "Hidden tabs release active transitions",
  );
  await page.evaluate(() => {
    delete document.hidden;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  const settle = () =>
    page.locator(".trait-panel-shell").evaluate(async (shell) => {
      await Promise.all(
        shell
          .getAnimations({ subtree: true })
          .map((a) => a.finished.catch(() => {})),
      );
    });
  for (const width of [320, 390, 1200]) {
    await page.setViewportSize({ width, height: width < 960 ? 700 : 560 });
    await button("Model").click();
    await settle();
    const choices = page.locator(".trait-options");
    await choices.evaluate((node) => (node.scrollTop = node.scrollHeight));
    assert.ok(await choices.evaluate((node) => node.scrollTop > 0));
    await choices.hover();
    const beforeWheel = await page.evaluate(() => ({
      page: scrollY,
      sidebar: document.querySelector(".market-sidebar").scrollTop,
    }));
    await page.mouse.wheel(0, 300);
    await page.evaluate(
      () =>
        new Promise((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(resolve)),
        ),
    );
    assert.deepEqual(
      await page.evaluate(() => ({
        page: scrollY,
        sidebar: document.querySelector(".market-sidebar").scrollTop,
      })),
      beforeWheel,
      "At the end of choices, wheel scrolling stays inside the picker",
    );
    await page.getByRole("searchbox").fill("Long model");
    assert.equal(
      await choices.evaluate((node) => node.scrollTop),
      0,
      "Search starts at the first result even when the new results still overflow",
    );
    await choices.evaluate((node) => {
      node.scrollTop = 80;
      node.querySelector(".trait-group-toggle").click();
    });
    assert.equal(
      await choices.evaluate((node) => node.scrollTop),
      80,
      "Selecting a group preserves the choices scroll position",
    );
    await page.getByRole("searchbox").fill("");
    await settle();
    const rapid = await page.evaluate(async () => {
      const shell = document.querySelector(".trait-panel-shell");
      const observerTargets = () => window.pickerResizeTargets.size;
      const frame = () => new Promise(requestAnimationFrame);
      for (const kind of [
        "backdrops",
        "patterns",
        "models",
        "patterns",
        "models",
      ]) {
        document.getElementById(`pick-${kind}`).click();
        await frame();
        await frame();
        if (document.querySelector(".trait-panel-shell") !== shell)
          return false;
        if (observerTargets() !== 0) return false;
        if (
          shell
            .getAnimations()
            .filter(
              (a) =>
                a.id === "trait-panel-visibility" && a.playState === "running",
            ).length > 1
        )
          return false;
      }
      return true;
    });
    assert.equal(
      rapid,
      true,
      "Rapid switches retain one shell and one fade without a resize observer",
    );
    await settle();
    const finalHeight = await page
      .locator(".trait-panel-shell")
      .evaluate((shell) => {
        const panel = shell.querySelector(".picker-panel");
        return Math.abs(
          shell.getBoundingClientRect().height -
            panel.getBoundingClientRect().height -
            parseFloat(getComputedStyle(panel).marginTop),
        );
      });
    assert.ok(
      finalHeight < 1,
      "Finished animation agrees with natural layout without a final jump",
    );
    await page.getByRole("searchbox").focus();
    await page.setViewportSize({ width: width + 16, height: 560 });
    await settle();
    assert.equal(
      await page
        .getByRole("searchbox")
        .evaluate((node) => node === document.activeElement),
      true,
      "Responsive resizing preserves search focus",
    );
    await page.evaluate(async () => {
      window.pickerShell = document.querySelector(".trait-panel-shell");
      document.getElementById("pick-models").click();
      await new Promise(requestAnimationFrame);
      document.getElementById("pick-backdrops").click();
    });
    await settle();
    assert.equal(
      await page.evaluate(
        () =>
          document.querySelector(".trait-panel-shell") === window.pickerShell,
      ),
      true,
      "Reopening during exit retains the shell at every screen size",
    );
    await button("Model").click();
    await settle();
    await page.getByRole("searchbox").focus();
    await button("Other control").focus();
    await page.keyboard.press("Escape");
    assert.equal(await button("Model").getAttribute("aria-expanded"), "true");
    assert.equal(
      await page.evaluate(() => document.activeElement.id),
      "other-control",
      "Escape outside a picker does not steal focus",
    );
    const clicks = await page.evaluate(() => window.otherClicks ?? 0);
    await button("Other control").click();
    await page.locator(".trait-panel-shell").waitFor({ state: "detached" });
    assert.equal(
      await page.evaluate(() => window.otherClicks),
      clicks + 1,
      "Outside click reaches its control before the picker collapses",
    );
    assert.equal(
      await page.evaluate(() => document.activeElement.id),
      "other-control",
    );
    await button("Model").click();
    await settle();
    await page.getByRole("searchbox").focus();
    const beforeEscape = await page.evaluate(() => ({
      page: scrollY,
      sidebar: document.querySelector(".market-sidebar").scrollTop,
    }));
    await page.keyboard.press("Escape");
    await page.locator(".trait-panel-shell").waitFor({ state: "detached" });
    assert.deepEqual(
      await page.evaluate(() => ({
        page: scrollY,
        sidebar: document.querySelector(".market-sidebar").scrollTop,
      })),
      beforeEscape,
      "Escape restores focus without scrolling the page or desktop sidebar",
    );
    assert.equal(
      await page.evaluate(() => document.activeElement.id),
      "pick-models",
    );
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
      false,
    );
  }
  await button("Model").click();
  await page.getByRole("searchbox").waitFor();
  await page.evaluate(() => window.unmount());
  assert.equal(await page.evaluate(() => window.pickerResizeTargets.size), 0);
  assert.equal(
    await page.evaluate(
      () =>
        window.pickerAnimations.filter(
          (a) => a.id.startsWith("trait-panel") && a.playState === "running",
        ).length,
    ),
    0,
    "Unmount cancels every picker transition",
  );
  assert.deepEqual(
    apiRequests,
    [],
    "Grouping and selection introduce no API requests",
  );
  assert.deepEqual(errors, []);
  const touch = await browser.newPage({
    viewport: { width: 390, height: 700 },
    isMobile: true,
    hasTouch: true,
    reducedMotion: "no-preference",
  });
  touch.on("pageerror", (error) => errors.push(error.message));
  const client = await touch.context().newCDPSession(touch);
  await client.send("Performance.enable");
  await client.send("Emulation.setCPUThrottlingRate", { rate: 4 });
  await touch.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__rarity`,
  );
  for (const name of ["Model", "Backdrop", "Symbol", "Model"]) {
    await touch.getByRole("button", { name, exact: true }).tap();
    assert.equal(
      await touch
        .getByRole("button", { name, exact: true })
        .getAttribute("aria-expanded"),
      "true",
    );
  }
  await touch.getByRole("button", { name: "Backdrop", exact: true }).tap();
  await touch.locator(".trait-panel-shell").evaluate(async (shell) => {
    await Promise.all(
      shell.getAnimations().map((a) => a.finished.catch(() => {})),
    );
  });
  await touch.evaluate(() => {
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (frames, options) {
      const result = animate.call(this, frames, options);
      if (
        this.matches(".trait-panel-shell") &&
        !window.pickerExit &&
        frames.at(-1).opacity === 0
      ) {
        result.pause();
        window.pickerExit = result;
      }
      return result;
    };
  });
  await touch.getByRole("button", { name: "Backdrop", exact: true }).tap();
  const layouts = async () =>
    (await client.send("Performance.getMetrics")).metrics.find(
      (metric) => metric.name === "LayoutCount",
    ).value;
  const baselineHeight = await touch
    .locator(".trait-panel-shell")
    .evaluate((shell) => shell.getBoundingClientRect().height);
  const beforeFade = await layouts();
  for (const time of [15, 30, 45, 60, 75]) {
    const height = await touch.evaluate(async (time) => {
      window.pickerExit.currentTime = time;
      await new Promise(requestAnimationFrame);
      return document
        .querySelector(".trait-panel-shell")
        .getBoundingClientRect().height;
    }, time);
    assert.equal(
      height,
      baselineHeight,
      "Closing picker retains stable layout throughout its fade",
    );
  }
  assert.equal(
    await layouts(),
    beforeFade,
    "Phone picker close does not relayout the market on each animation frame",
  );
  await touch.evaluate(() => window.pickerExit.finish());
  await touch.locator(".trait-panel-shell").waitFor({ state: "detached" });
  await client.send("Emulation.setCPUThrottlingRate", { rate: 1 });
  await touch.getByRole("button", { name: "Model", exact: true }).tap();
  await touch.getByRole("searchbox").fill("Long model");
  await touch.getByRole("button", { name: "Other control", exact: true }).tap();
  await touch.locator(".trait-panel-shell").waitFor({ state: "detached" });
  assert.equal(await touch.evaluate(() => window.otherClicks), 1);
  assert.deepEqual(errors, []);
  await touch.close();
  const mediaPage = await browser.newPage({
    viewport: { width: 390, height: 700 },
    hasTouch: true,
    isMobile: true,
    reducedMotion: "reduce",
  });
  const mediaErrors = [],
    mediaRequests = [];
  mediaPage.on("pageerror", (error) => mediaErrors.push(error.message));
  mediaPage.on("request", (request) =>
    mediaRequests.push(new URL(request.url()).pathname),
  );
  await mediaPage.addInitScript(() => {
    window.pickerObserved = new Map();
    const Native = IntersectionObserver;
    window.IntersectionObserver = class extends Native {
      targets = new Set();
      observe(target) {
        this.targets.add(target);
        window.pickerObserved.set(
          target,
          (window.pickerObserved.get(target) || 0) + 1,
        );
        super.observe(target);
      }
      unobserve(target) {
        if (this.targets.delete(target)) {
          const count = window.pickerObserved.get(target) - 1;
          if (count) window.pickerObserved.set(target, count);
          else window.pickerObserved.delete(target);
        }
        super.unobserve(target);
      }
      disconnect() {
        for (const target of [...this.targets]) this.unobserve(target);
        super.disconnect();
      }
    };
  });
  const mediaClient = await mediaPage.context().newCDPSession(mediaPage);
  await mediaClient.send("Emulation.setCPUThrottlingRate", { rate: 4 });
  await mediaPage.goto(server.resolvedUrls.local[0] + "__rarity?media=1");
  assert.equal(
    await mediaPage.locator(".option-art img").count(),
    0,
    "Closed pickers do not mount media",
  );
  await mediaPage.getByRole("button", { name: "Model", exact: true }).tap();
  const readyIcons = () =>
    mediaPage.waitForFunction(() => {
      const images = [...document.querySelectorAll(".trait-options img")];
      return (
        images.length > 1 &&
        images.every(
          (image) =>
            image.complete &&
            image.naturalWidth > 0 &&
            image.classList.contains("loaded"),
        )
      );
    });
  await readyIcons();
  const mounted = await mediaPage.locator(".trait-options img").count();
  assert(
    mounted < 40,
    `Only nearby choices mount images, got ${mounted} out of 204 Primary models`,
  );
  assert.equal(
    await mediaPage.locator(".trait-options .model-animation").count(),
    0,
  );
  const ahead = await mediaPage.locator(".trait-options").evaluate((root) => {
    const edge = root.getBoundingClientRect().bottom;
    return [...root.querySelectorAll(".trait-option[aria-label]")]
      .find((button) => {
        const box = button.getBoundingClientRect();
        return box.top > edge + 4 && box.bottom < edge + 90;
      })
      ?.getAttribute("aria-label");
  });
  assert(ahead, "Fixture includes a choice just below the picker viewport");
  await mediaPage
    .getByRole("button", { name: ahead, exact: true })
    .locator("img.loaded")
    .waitFor({ state: "attached" });
  assert.equal(
    await mediaPage.evaluate(
      () =>
        [...window.pickerObserved].filter(([target]) =>
          target.matches(".gift-preview"),
        ).length,
    ),
    0,
    "Static icons have no second visibility gate or player observers",
  );
  assert(
    await mediaPage.evaluate(() =>
      [...window.pickerObserved.values()].every((count) => count === 1),
    ),
    "Each media target is observed once",
  );
  const tail = mediaPage.getByRole("button", {
    name: "Long model name 199",
    exact: true,
  });
  assert.equal(
    await tail.locator("img").count(),
    0,
    "The unvisited tail remains unloaded",
  );
  await tail.scrollIntoViewIfNeeded();
  await tail.locator("img.loaded").waitFor();
  await mediaPage
    .locator(".trait-options")
    .evaluate((root) => (root.scrollTop = 0));
  await readyIcons();
  const beforeReopen = new Map(mediaReads);
  await mediaPage.getByRole("button", { name: "Model", exact: true }).tap();
  await mediaPage.locator(".trait-panel-shell").waitFor({ state: "detached" });
  assert.equal(
    await mediaPage.evaluate(() => window.pickerObserved.size),
    0,
    "Closing releases picker visibility observers",
  );
  await mediaPage.getByRole("button", { name: "Model", exact: true }).tap();
  await readyIcons();
  assert.deepEqual(
    mediaReads,
    beforeReopen,
    "Reopening reuses the HTTP image cache without another transfer",
  );
  for (const name of ["Backdrop", "Symbol", "Model"])
    await mediaPage.getByRole("button", { name, exact: true }).tap();
  await mediaPage.getByRole("searchbox").fill("Long model name 199");
  assert.equal(
    await mediaPage.locator(".trait-group-options .trait-option").count(),
    1,
    "Rapid switching leaves search scoped to the final picker",
  );
  await mediaPage.evaluate(() => window.unmount());
  assert.equal(
    await mediaPage.evaluate(() => window.pickerObserved.size),
    0,
    "Unmount releases root and viewport observers",
  );
  assert(
    !mediaRequests.some(
      (path) =>
        path.endsWith(".tgs") ||
        /^\/(catalog|collections|valuations|ws)(\/|$)/.test(path),
    ),
    "Picker interactions start no animation or API requests",
  );
  assert.deepEqual(mediaErrors, []);
  await mediaPage.close();
  console.log(
    `Rarity picker browser passed: rapid switching, search and focus/scroll preservation, exact rarity groups, interrupted animations, 4× CPU touch checks, ${mounted}/204 Primary icons admitted initially, scroll-root preloading, warm HTTP reuse, observer cleanup and no animation/API requests.`,
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
