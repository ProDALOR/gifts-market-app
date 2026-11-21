import assert from "node:assert/strict";
import { mkdtemp, rm, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
import { floorReport } from "./valuation-fixture.mjs";
const root = fileURLToPath(new URL("..", import.meta.url));
const cacheDir = await mkdtemp(join(tmpdir(), "attribute-values-"));
const html = `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"></head><body><div id="fixture"></div><script type="module">
import React,{useState}from'react';import{createRoot}from'react-dom/client';
import{FloorsProvider}from'/src/valuation/floors.tsx';import{CollectionRail}from'/src/collections/collection-picker.tsx';import{TraitPickers}from'/src/collections/trait-pickers.tsx';
const media={preview_url:'/art.svg',thumbnail_url:'/art.svg'};
const collections=[{id:'a',name:'Intelligence Cup',preview:media,model_ids:['a-model'],backdrop_ids:['a-back'],pattern_ids:[]},{id:'b',name:'Resistance Dog',preview:media,model_ids:['b-model'],backdrop_ids:['b-back'],pattern_ids:[]}];
const collectionIds=collections.map(c=>c.id);
const options=new Map(collections.flatMap(c=>[[c.id+'-model',{id:c.id+'-model',name:'Very long '+c.name+' model with an unbrokenlongnamethatmustfit',kind:'model',media}],[c.id+'-back',{id:c.id+'-back',name:'Long '+c.name+' backdrop',kind:'backdrop',palette:{center:'#664455',edge:'#332233',pattern:'#222222'}}]]));
function App(){const[selected,select]=useState('a'),[open,setOpen]=useState(),[filters,change]=useState({});window.chooseCollection=select;const collection=collections.find(c=>c.id===selected);const scoped=React.useMemo(()=>new Map([...options].filter(([id])=>id.startsWith(selected+'-'))),[selected]);return React.createElement(FloorsProvider,{currency:'GRAM',collection:selected,collections:collections.map(c=>c.id)},React.createElement('div',{className:'workspace'},React.createElement('main',{className:'orderbook market-shell'},React.createElement('aside',{className:'market-sidebar'},React.createElement(CollectionRail,{collections,selected,select}),React.createElement(TraitPickers,{collection,options:scoped,filters,open,setOpen,change:(key,value)=>change({...filters,[key]:value})})),React.createElement('section',{className:'market-results'},'Values test'))));}
createRoot(document.getElementById('fixture')).render(React.createElement(App));
</script></body></html>`;
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "attribute-values-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url === "/__attribute_values") {
            res.setHeader("Content-Type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
            return;
          }
          if (req.url === "/art.svg") {
            res.setHeader("Content-Type", "image/svg+xml");
            res.end(
              '<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><circle cx="50" cy="50" r="40" fill="#edc276"/></svg>',
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
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const commands = [];
  let hold = false,
    held = [];
  await page.routeWebSocket("**/ws", (socket) => {
    socket.send(JSON.stringify({ type: "market.status", status: "available" }));
    socket.onMessage((raw) => {
      const command = JSON.parse(String(raw));
      if (command.type === "cancel") return;
      assert.equal(command.type, "request");
      assert.equal(command.method, "valuation.query");
      commands.push(command);
      const selected = command.params.target.details?.collection_id ?? "a";
      const data = floorReport(
        command.params,
        [selected + "-model"],
        [selected + "-back"],
      );
      for (const row of data.results[0].data.collections) {
        row.floor.price.value = "12345678.123456789";
        if (row.traits) {
          row.traits.models[selected + "-model"].price.value =
            selected === "a" ? "10.123456789" : "20.123456789";
        }
      }
      const send = () =>
        socket.send(
          JSON.stringify({
            type: "result",
            id: command.id,
            method: "valuation.query",
            data,
          }),
        );
      if (hold) held.push({ send, command });
      else send();
    });
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__attribute_values`,
  );
  await page.locator(".collection-floor .floor-badge").first().waitFor();
  const geometry = async (desktop) => {
    const layout = await page.locator(".collection-list").evaluate(
      (list, desktop) =>
        [...list.querySelectorAll(".collection-tile")].map((tile) => {
          const box = tile.getBoundingClientRect(),
            art = tile.querySelector(".collection-art").getBoundingClientRect(),
            name = tile.querySelector("b").getBoundingClientRect(),
            price = tile
              .querySelector(".collection-floor")
              .getBoundingClientRect();
          return {
            contained:
              price.left >= box.left &&
              price.right <= box.right &&
              price.top >= box.top &&
              price.bottom <= box.bottom,
            separate: desktop
              ? price.left >= name.right - 1 && name.left >= art.right - 1
              : tile.getAttribute("aria-pressed") === "true"
                ? price.top >= name.bottom - 1
                : price.top >= art.bottom - 1,
          };
        }),
      desktop,
    );
    assert.ok(
      layout.every((row) => row.contained && row.separate),
      JSON.stringify(layout),
    );
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
      false,
    );
  };
  assert.deepEqual(
    await page.locator(".collection-floor .quote-amount").allTextContents(),
    ["12.35M", "12.35M"],
  );
  assert.ok(
    (
      await page
        .locator(".collection-floor .floor-badge")
        .first()
        .getAttribute("title")
    ).includes("12345678.123456789"),
  );
  const pricesFit = async () =>
    assert.ok(
      await page
        .locator(".collection-floor .quote-amount")
        .evaluateAll((nodes) =>
          nodes.every((node) => node.scrollWidth <= node.clientWidth),
        ),
      "Large collection floors remain fully visible",
    );
  await pricesFit();
  await geometry(false);
  await page.getByRole("button", { name: "Model", exact: true }).click();
  await page.locator(".trait-option-value .floor-badge").waitFor();
  const checkTrait = async () =>
    assert.equal(
      await page
        .locator(".trait-option:has(.trait-option-value)")
        .evaluate((button) => {
          const box = button.getBoundingClientRect(),
            art = button.querySelector(".option-art").getBoundingClientRect(),
            name = button
              .querySelector(".trait-option-name")
              .getBoundingClientRect(),
            price = button
              .querySelector(".trait-option-value")
              .getBoundingClientRect();
          return (
            name.left >= art.right &&
            price.left >= name.left &&
            price.top >= name.bottom &&
            price.right <= box.right &&
            price.top >= box.top &&
            price.bottom <= box.bottom
          );
        }),
      true,
    );
  await checkTrait();
  await page.getByRole("button", { name: "Backdrop", exact: true }).click();
  await page.locator(".trait-option-value .floor-badge").waitFor();
  await checkTrait();
  await page.getByRole("button", { name: "Model", exact: true }).click();
  const requestsBeforeResize = commands.length;
  await page.setViewportSize({ width: 1200, height: 900 });
  await geometry(true);
  await pricesFit();
  await checkTrait();
  await page.setViewportSize({ width: 320, height: 844 });
  await geometry(false);
  await checkTrait();
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Search collections" }).click();
  await page.locator(".collection-rail.searching").waitFor();
  assert.equal(
    await page.locator(".collection-tile").evaluateAll((tiles) =>
      tiles.every((tile) => {
        const name = tile.querySelector("b").getBoundingClientRect(),
          price = tile
            .querySelector(".collection-floor")
            .getBoundingClientRect();
        return price.top >= name.bottom - 1;
      }),
    ),
    true,
  );
  await page.getByRole("button", { name: "Close collection search" }).click();
  await page.setViewportSize({ width: 1200, height: 900 });
  await geometry(true);
  await page.getByRole("button", { name: "Model", exact: true }).click();
  await page.locator(".trait-option-value .quote-amount").waitFor();
  assert.equal(
    await page.locator(".trait-option-value .quote-amount").textContent(),
    "10.12",
  );
  assert.ok(
    (
      await page
        .locator(".trait-option-value .floor-badge")
        .getAttribute("title")
    ).includes("10.123456789"),
    "Tooltips retain the exact floor",
  );
  hold = true;
  await page.evaluate(() => window.chooseCollection("b"));
  for (let i = 0; !held.length && i < 100; i++)
    await new Promise((r) => setTimeout(r, 10));
  assert.ok(held.length);
  const b = held.at(-1);
  assert.equal(b.command.params.target.details.collection_id, "b");
  assert.equal(
    await page.locator(".trait-option-value .quote-amount").count(),
    0,
    "Old collection values are not shown for new IDs",
  );
  hold = false;
  await page.evaluate(() => window.chooseCollection("a"));
  await page.waitForFunction(
    () =>
      document.querySelector(".trait-option-value .quote-amount")
        ?.textContent === "10.12",
  );
  b.send();
  assert.equal(
    await page.locator(".trait-option-value .quote-amount").textContent(),
    "10.12",
  );
  assert.ok(
    commands.length <= requestsBeforeResize + 4,
    "Resizing and picker state do not start per-option queries",
  );
  await mkdir("/tmp/gifts-market-ui", { recursive: true });
  await page.screenshot({
    path: "/tmp/gifts-market-ui/attribute-picker-desktop.png",
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: "/tmp/gifts-market-ui/attribute-picker-mobile.png",
  });
  assert.deepEqual(errors, []);
  console.log(
    "Attribute values browser passed: mobile/desktop price placement, long labels, search layout, scope switching, delayed frames and batched requests.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
