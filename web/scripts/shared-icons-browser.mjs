// Isolated static-icon rendering, HTTP cache reuse and recovery; no Telegram connection.
import assert from "node:assert/strict";
import { readFile, readdir, mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const registry = JSON.parse(
  await readFile(
    new URL("../../crates/market-media/assets/icons.json", import.meta.url),
  ),
);
const manifest = registry.icons;
const cacheDir = await mkdtemp(join(tmpdir(), "market-icons-"));
const requests = new Map();
let recovering = false;
const svg =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path fill="#fff" d="M64 64H448V448H64Z"/></svg>';
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module">
import React from 'react';import{createRoot}from'react-dom/client';
import{SharedIcon}from'/src/ui/icons.tsx';import{useState}from'react';import'/src/style.css';import'/src/app/navigation.css';
import{stillImages}from'/src/previews/still-image-cache.ts';
const retained=new Image();retained.src='data:image/svg+xml,'+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512"/>');await retained.decode();
for(let i=0;i<24;i++)stillImages.remember('/retained-gift-'+i,retained);
window.giftRetained=()=>stillImages.has('/retained-gift-0');
const names=${JSON.stringify(Object.keys(manifest))};
const h=React.createElement;
function App(){const[visible,show]=useState(true);const[name,choose]=useState('collection');return h(React.Fragment,null,
h('button',{onClick:()=>show(!visible)},'Toggle'),h('button',{onClick:()=>choose(name==='collection'?'model':'collection')},'Switch'),
h('div',{id:'changing'},h(SharedIcon,{name})),
h('div',{id:'sheet',style:{display:'grid',gridTemplateColumns:'repeat(auto-fit,minmax(150px,1fr))',gap:16,padding:20}},names.filter(n=>n!=='warning').map(name=>h('div',{key:name,style:{display:'flex',alignItems:'center',gap:8}},h(SharedIcon,{name,label:name,className:'sample'}),name))),
h('div',{id:'duplicates'},Array.from({length:50},(_,i)=>h(SharedIcon,{key:i,name:'collection'}))),
visible&&h('div',{id:'recovering',style:{marginTop:1000}},h(SharedIcon,{name:'warning',label:'Warning'})),
h('div',{id:'controls'},
h('nav',{className:'app-navigation'},h('a',{href:'#'},h(SharedIcon,{name:'market'}),'Market')),
h('button',{className:'account-button'},h(SharedIcon,{name:'user'})),
h('div',{className:'search-tile open'},h(SharedIcon,{name:'search',className:'search-icon'})),
h('div',{className:'search-tile',id:'closed-search'},h(SharedIcon,{name:'search',className:'search-icon'})),
h('div',{className:'currency-picker',style:{width:58}},h('button',{className:'picker-trigger'},h(SharedIcon,{name:'gram',className:'currency-symbol'}),h('svg',{className:'picker-chevron'}))),
h('div',{className:'valuation-value'},h(SharedIcon,{name:'gram',className:'currency-symbol'})),
h('ul',{className:'valuation-metrics-compact'},h('li',{className:'valuation-value'},h(SharedIcon,{name:'gram',className:'currency-symbol'})))))}
createRoot(document.getElementById('root')).render(h(App));
</script><style>.sample{width:32px;height:32px}#sheet{color:#eee;background:#252229}</style></body></html>`;
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "shared-icons-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url === "/__icons") {
            res.setHeader("Content-Type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
            return;
          }
          if (!req.url?.startsWith("/media/")) return next();
          const path = req.url.split("?")[0];
          requests.set(path, (requests.get(path) || 0) + 1);
          const name = /^\/media\/icons\/([^/]+)\/preview$/.exec(path)?.[1];
          if (!name || !manifest[name]) {
            res.statusCode = 404;
            res.end();
            return;
          }
          if (name === "warning" && !recovering) {
            res.statusCode = 503;
            res.setHeader("Retry-After", "2");
            res.end();
            return;
          }
          let body = svg,
            type = "image/svg+xml";
          if (process.env.MARKET_TEST_MEDIA_DIR) {
            const icon = manifest[name],
              files = await readdir(process.env.MARKET_TEST_MEDIA_DIR);
            const file = files
              .filter(
                (f) =>
                  f.startsWith(icon.document_id + ".preview-") &&
                  f.endsWith(".svg"),
              )
              .sort()
              .at(-1);
            assert.ok(file, `Missing cached ${name}`);
            body = await readFile(
              join(process.env.MARKET_TEST_MEDIA_DIR, file),
            );
          }
          res.setHeader("Content-Type", type);
          res.setHeader("Cache-Control", "public,max-age=86400");
          res.end(body);
        });
      },
    },
  ],
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  for (const width of [390, 1440]) {
    requests.clear();
    recovering = false;
    const page = await browser.newPage({ viewport: { width, height: 900 } }),
      errors = [];
    page.setDefaultTimeout(10000);
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto(server.resolvedUrls.local[0] + "__icons");
    console.log(`Checking ${width}px icons`);
    await page.waitForFunction(
      (count) =>
        document.querySelectorAll("#sheet .shared-icon-mask").length === count,
      Object.keys(manifest).length - 1,
    );
    assert.equal(
      requests.get("/media/icons/collection/preview"),
      1,
      "repeated icons share HTTP caching",
    );
    assert.equal(await page.locator("canvas").count(), 0);
    assert.equal(
      await page.evaluate(() => window.giftRetained()),
      true,
      "all interface icons leave the full gift retention budget intact",
    );
    for (const [selector, size] of [
      ["#controls .app-navigation .shared-icon", width < 960 ? 20 : 18],
      ["#controls .account-button .shared-icon", 22],
      ["#controls .search-tile.open .search-icon", 17],
      ["#closed-search .search-icon", 23],
      ["#controls .currency-picker .shared-icon", 20],
      ["#controls > .valuation-value .currency-symbol", 14],
      ["#controls .valuation-metrics-compact .currency-symbol", 12],
    ]) {
      const bounds = await page.locator(selector).boundingBox();
      assert.equal(bounds.width, size, `${selector} width at ${width}px`);
      assert.equal(bounds.height, size, `${selector} height at ${width}px`);
    }
    assert.ok(
      [...requests.keys()].every((url) => url.endsWith("/preview")),
      "no TGS/animation requests",
    );
    for (let i = 0; i < 5; i++)
      await page.getByRole("button", { name: "Switch", exact: true }).click();
    await page.waitForFunction(
      () =>
        document
          .querySelector("#changing img")
          ?.getAttribute("src")
          ?.startsWith("/media/icons/model/preview?v=") &&
        document.querySelector("#changing .shared-icon-mask"),
    );
    await page.waitForFunction(() =>
      document.querySelector("#recovering .shared-icon-fallback"),
    );
    await page.locator("#recovering").scrollIntoViewIfNeeded();
    await page.waitForTimeout(150);
    const before = requests.get("/media/icons/warning/preview");
    for (let i = 0; i < 4; i++)
      await page.getByRole("button", { name: "Toggle", exact: true }).click();
    assert.equal(
      requests.get("/media/icons/warning/preview"),
      before,
      "remount respects shared failure cooldown",
    );
    await page.evaluate(() => scrollTo(0, 0));
    await page.waitForTimeout(2300);
    assert.equal(
      requests.get("/media/icons/warning/preview"),
      before,
      "offscreen icons do not retry after their cooldown expires",
    );
    recovering = true;
    await page.locator("#recovering").scrollIntoViewIfNeeded();
    await page.waitForFunction(
      () => document.querySelector("#recovering .shared-icon-mask"),
      {},
      { timeout: 10000 },
    );
    assert.deepEqual(errors, []);
    assert.equal(
      await page.locator("body").evaluate((el) => el.scrollWidth <= innerWidth),
      true,
    );
    const output = fileURLToPath(new URL("../../.market/run", import.meta.url));
    await mkdir(output, { recursive: true });
    await page.screenshot({
      path: join(output, `shared-icons-${width}.png`),
      fullPage: true,
    });
    await page.close();
  }
  console.log(
    "Shared icons: mobile/desktop, static rendering, HTTP reuse, switching and retry cooldowns passed.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
