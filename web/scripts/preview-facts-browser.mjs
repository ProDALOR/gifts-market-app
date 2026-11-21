import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const cacheDir = await mkdtemp(join(tmpdir(), "preview-facts-"));
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/src/style.css"></head><body><div id="fixture"></div><script type="module">
import React,{useState}from'react';import{createRoot}from'react-dom/client';
import{FloorDetails}from'/src/orderbook/floor-details.tsx';import{ExecutionTime}from'/src/orderbook/execution-time.tsx';import{OrderValuationSummary}from'/src/orderbook/order-valuation.tsx';
const at=new Date(Date.now()-3*3600000).toISOString();
const result={strategy:'fixture',revision:'r1',data:{floor:{value:'12.50'}}};
const strategy={id:'fixture',name:'Gift floor',description:'Fixture',input:{targets:['gift'],period:'unsupported'},documentation_url:'/guide',documentation_revision:'1',revision:'r1',display:{fields:[{id:'floor',label:'Estimate',primary:true,format:{kind:'currency',decimals:2}}],evidence:[],note:''}};
const fields=[{id:'floor',label:'Estimate',primary:true,value:{kind:'currency',amount:'12.50',currency_id:'GRAM'}}];
const state={key:'fixture',report:{target:{kind:'gift',gift:{id:'g',number:1,collection_id:'c',traits:{}}},currency_id:'GRAM',evaluated_at:at,stale:false,results:[result]},valuations:[{strategy,result,fields,evidence:[],primaryFields:fields,stale:false}],available:true,loading:false};
function App(){const[kind,setKind]=useState('value');window.setFloorKind=setKind;const floor=kind==='value'?{price:{value:'10.000000001'},evidence:{gifts:2,listings:4}}:kind==='empty'?{price:{error:{code:'not_enough_data',message:'No eligible asks match this selection'}},evidence:{gifts:0,listings:0}}:undefined;
return React.createElement('main',{className:'workspace'},React.createElement('section',{className:'detail-body'},React.createElement(ExecutionTime,{at}),React.createElement(OrderValuationSummary,{state}),React.createElement(FloorDetails,{label:'Model floor',name:'Dogfather',floor,currency:'GRAM',loading:kind==='loading',error:kind==='error'?'Attribute floors could not be loaded':''})));}
createRoot(document.getElementById('fixture')).render(React.createElement(App));
</script></body></html>`;
const server = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "preview-facts-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url !== "/__facts") return next();
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
  const page = await browser.newPage({
    viewport: { width: 320, height: 700 },
    hasTouch: true,
    isMobile: true,
    reducedMotion: "reduce",
  });
  page.setDefaultTimeout(10_000);
  const errors = [],
    requests = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("request", (req) => {
    if (/^\/(valuations|catalog|ws)(\/|$|\?)/.test(new URL(req.url()).pathname))
      requests.push(req.url());
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__facts`,
  );
  const floor = page.getByRole("button", {
    name: "View model floor details for Dogfather",
    exact: true,
  });
  await floor.waitFor();
  assert.equal(
    await page.evaluate(() => matchMedia("(pointer: coarse)").matches),
    true,
  );
  for (const selector of [
    ".execution-time-trigger",
    ".valuation-info-button",
    ".floor-inspect",
  ]) {
    assert.ok(
      await page.locator(selector).evaluate((el) => {
        const bounds = el.getBoundingClientRect();
        return bounds.width >= 44 && bounds.height >= 44;
      }),
      `${selector} provides a touch target of at least 44px`,
    );
  }
  await floor.tap();
  const popup = page.getByRole("dialog", {
    name: "Floor details",
    exact: true,
  });
  await popup.waitFor();
  assert.equal(
    await popup.locator(".detail-fact-price .quote-amount").textContent(),
    "10.000000001",
  );
  await page.evaluate(() => window.setFloorKind("empty"));
  await popup
    .getByText("No eligible asks match this selection", { exact: true })
    .waitFor();
  assert.deepEqual(
    await popup.locator("dd").allTextContents(),
    ["0", "0"],
    "Open floor details follow the current cached evidence",
  );
  await page
    .getByRole("button", { name: "Close floor details", exact: true })
    .tap();
  await popup.waitFor({ state: "detached" });
  await page.evaluate(() => window.setFloorKind("loading"));
  await page
    .getByRole("status", { name: "Loading model floor", exact: true })
    .waitFor();
  assert.equal(await floor.isDisabled(), true);
  await page.evaluate(() => window.setFloorKind("error"));
  await page
    .getByLabel("Model floor could not be loaded", { exact: true })
    .waitFor();
  await floor.tap();
  await popup
    .getByText("Attribute floors could not be loaded", { exact: true })
    .waitFor();
  await page
    .getByRole("button", { name: "Close floor details", exact: true })
    .tap();
  await page
    .getByRole("button", { name: "View execution time", exact: true })
    .tap();
  const time = page.getByRole("dialog", {
    name: "Execution time",
    exact: true,
  });
  await time.waitFor();
  assert.ok(await time.locator("time").getAttribute("datetime"));
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
    true,
  );
  await page
    .getByRole("button", { name: "Close execution time", exact: true })
    .tap();
  assert.deepEqual(requests, [], "Fact popups never read market data");
  assert.deepEqual(errors, []);
  console.log(
    "Preview fact browser passed: native touch targets, exact evidence, live popup updates, loading/empty/error states, execution timestamp and no market requests.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
