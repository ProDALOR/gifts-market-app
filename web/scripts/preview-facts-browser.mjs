import assert from "node:assert/strict";
import { mkdtemp, rm, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const cacheDir = await mkdtemp(join(tmpdir(), "preview-facts-"));
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/src/style.css"></head><body><div id="fixture"></div><script type="module">
import React,{useState}from'react';import{createRoot}from'react-dom/client';
import{Quote}from'/src/ui/market-controls.tsx';import{FloorValue}from'/src/orderbook/floor-value.tsx';import{ExecutionTime}from'/src/orderbook/execution-time.tsx';import{OrderValuationSummary}from'/src/orderbook/order-valuation.tsx';
const at=new Date(Date.now()-3*3600000).toISOString();
const result={strategy:'fixture',revision:'r1',data:{floor:{value:'12.50'}}};
const strategy={id:'fixture',name:'Gift floor',description:'Fixture',purpose:'estimate',input:{targets:['gift'],period:'unsupported'},documentation_url:'/guide',documentation_revision:'1',revision:'r1',display:{fields:[{id:'floor',label:'Estimate',primary:true,format:{kind:'currency',decimals:2}}],evidence:[],note:''}};
const fields=[{id:'floor',label:'Estimate',primary:true,value:{kind:'currency',amount:'12.50',currency_id:'GRAM'}}];
const state={key:'fixture',report:{target:{kind:'gift',gift:{id:'g',number:1,collection_id:'c',traits:{}}},currency_id:'GRAM',evaluated_at:at,stale:false,results:[result]},valuations:[{strategy,result,fields,evidence:[],primaryFields:fields,stale:false}],available:true,loading:false};
function App(){const[kind,setKind]=useState('value'),[view,setView]=useState({mode:'ask',price:'12.50',amount:'12.50',multiple:false,stale:false});window.setEstimateView=value=>setView(previous=>({...previous,...value}));
const shownFields=fields.map(field=>({...field,value:{...field.value,amount:view.amount}}));
const valuations=[{...state.valuations[0],fields:shownFields,primaryFields:shownFields,stale:view.stale}];
if(view.multiple)valuations.push({...valuations[0],strategy:{...strategy,id:'second',name:'Ask floor'}});
const shownState={...state,key:view.mode,valuations};
window.setFloorKind=setKind;const floor=kind==='value'?{value:'10.000000001',count:2}:kind==='empty'?{error:{code:'not_enough_data',message:'No eligible asks match this selection'},count:0}:undefined;
return React.createElement('main',{className:'workspace'},React.createElement('section',{className:'detail-body'},React.createElement(ExecutionTime,{at}),React.createElement('div',{className:'detail-pricing'},React.createElement('section',{className:'listing-price'},React.createElement('div',{className:'listing-price-row'},React.createElement('h3',null,view.mode==='ask'?'Ask price':view.mode==='bid'?'Bid price':'Paid'),React.createElement('div',{className:'detail-price'},React.createElement(Quote,{price:view.price,currency:'GRAM'})))),React.createElement(OrderValuationSummary,{state:shownState,criteria:view.mode==='bid',history:view.mode==='history',price:view.mode==='ask'?view.price:undefined,currency:'GRAM'})),React.createElement(FloorValue,{label:'Model floor',floor,currency:'GRAM',loading:kind==='loading',error:kind==='error'?'Attribute floors could not be loaded':''})));}
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
          if (req.url.startsWith("/media/icons/")) {
            res.setHeader("Content-Type", "image/svg+xml");
            res.end(
              '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/></svg>',
            );
            return;
          }
          if (req.url === "/guide") {
            res.setHeader("Content-Type", "text/markdown");
            res.end("# Calculation\n\nThe full calculation remains readable.");
            return;
          }
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
  const floor = page.locator(".floor-value");
  await floor.waitFor();
  assert.equal(
    await page.evaluate(() => matchMedia("(pointer: coarse)").matches),
    true,
  );
  for (const selector of [
    ".execution-time-trigger",
    ".valuation-info-button",
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
  assert.equal(await floor.evaluate((el) => el.tagName), "SPAN");
  assert.equal(await floor.locator(".quote-amount").textContent(), "10");
  assert.equal(
    await page.getByRole("dialog").count(),
    0,
    "Floors are read-only",
  );
  await page.evaluate(() => window.setFloorKind("empty"));
  await floor.getByText("0 gifts", { exact: true }).waitFor();
  await page.evaluate(() => window.setFloorKind("loading"));
  await page
    .getByRole("status", { name: "Loading model floor", exact: true })
    .waitFor();
  await page.evaluate(() => window.setFloorKind("error"));
  await page
    .getByLabel("Model floor could not be loaded", { exact: true })
    .waitFor();
  await floor.tap();
  assert.equal(await page.getByRole("dialog").count(), 0);
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
  await mkdir("/tmp/gifts-market-ui", { recursive: true });
  const checkCompact = async (page, touch) => {
    for (const width of [320, 390, 1200]) {
      await page.setViewportSize({ width, height: 800 });
      for (const mode of ["ask", "bid", "history"]) {
        await page.evaluate(
          (mode) =>
            window.setEstimateView({
              mode,
              multiple: true,
              amount: "12.50",
              price: "12.50",
              stale: false,
            }),
          mode,
        );
        await page.waitForFunction(
          () =>
            document.querySelectorAll(".valuation-strategy-row").length === 2,
        );
        await page
          .getByRole("heading", {
            name:
              mode === "ask"
                ? "Ask price"
                : mode === "bid"
                  ? "Bid price"
                  : "Paid",
            exact: true,
          })
          .waitFor();
        assert.equal(
          await page.locator(".valuation-strategy-row").count(),
          2,
          "Equal strategy values stay separate",
        );
        assert.equal(await page.locator(".price-comparison").count(), 0);
        assert.equal(
          await page.locator(".valuation-comparison-space").count(),
          0,
        );
        const layout = await page
          .locator(".detail-pricing")
          .evaluate((panel) => ({
            overflow: document.documentElement.scrollWidth > innerWidth,
            rows: [...panel.querySelectorAll(".valuation-result-row")].map(
              (row) => {
                const value = row.querySelector(".valuation-value");
                const heading = row.querySelector(".valuation-heading");
                const icon = row.querySelector(".valuation-info-button");
                return {
                  height: row.getBoundingClientRect().height,
                  font: getComputedStyle(value).fontSize,
                  icon: getComputedStyle(
                    value.querySelector(".currency-symbol"),
                  ).width,
                  separated:
                    heading.getBoundingClientRect().right <=
                    value.getBoundingClientRect().left,
                  touchTarget:
                    icon.getBoundingClientRect().width >= 44 &&
                    icon.getBoundingClientRect().height >= 44,
                };
              },
            ),
          }));
        assert.equal(layout.overflow, false);
        for (const row of layout.rows) {
          assert.equal(row.font, "14px");
          assert.equal(row.icon, "12px");
          assert.equal(row.separated, true);
          assert.ok(
            row.height <= (touch ? 44 : 28),
            JSON.stringify({ width, mode, touch, row }),
          );
          if (touch) assert.equal(row.touchTarget, true);
        }
        if (width === 390)
          await page.screenshot({
            path: `/tmp/gifts-market-ui/compact-estimates-${mode}-${touch ? "touch" : "desktop"}.png`,
          });
      }
    }
    await page.evaluate(() =>
      window.setEstimateView({ mode: "ask", price: "10", amount: "12.50" }),
    );
    await page
      .getByLabel("Ask price is about 20% below this estimate", { exact: true })
      .first()
      .waitFor();
    assert.equal(
      await page.locator(".price-comparison").first().textContent(),
      "−20%",
    );
    assert.equal(
      await page.locator(".price-comparison").first().getAttribute("title"),
      "Ask price is about 20% below this estimate",
    );
    const inline = await page
      .locator(".valuation-primary-result")
      .first()
      .evaluate((row) => {
        const price = row
            .querySelector(".valuation-value")
            .getBoundingClientRect(),
          delta = row
            .querySelector(".price-comparison")
            .getBoundingClientRect();
        return (
          Math.abs(
            (price.top + price.bottom) / 2 - (delta.top + delta.bottom) / 2,
          ) < 1
        );
      });
    assert.equal(inline, true, "Meaningful deltas stay inline");
    await page.evaluate(() => window.setEstimateView({ stale: true }));
    await page.waitForFunction(
      () => document.querySelectorAll(".price-comparison").length === 0,
    );
    assert.equal(
      await page.locator(".price-comparison").count(),
      0,
      "Stale estimates have no comparison badge",
    );
    await page.evaluate(() =>
      window.setEstimateView({ stale: false, price: "12.49999" }),
    );
    assert.equal(
      await page.locator(".price-comparison").count(),
      0,
      "Near-equal prices have no comparison badge",
    );
    await page
      .getByRole("button", {
        name: "How is Gift floor calculated?",
        exact: true,
      })
      .click();
    const popup = page.getByRole("dialog", {
      name: "Calculation guide",
      exact: true,
    });
    await popup.waitFor();
    await popup
      .getByText("The full calculation remains readable.", { exact: true })
      .waitFor();
    const popupLayout = await popup
      .locator(".valuation-metrics")
      .evaluate((list) => ({
        compact: list.classList.contains("valuation-metrics-compact"),
        value: getComputedStyle(list.querySelector(".valuation-value"))
          .fontSize,
        padding: getComputedStyle(list.querySelector("li")).paddingTop,
        columns: getComputedStyle(list).gridTemplateColumns.split(" ").length,
      }));
    assert.deepEqual(
      popupLayout,
      { compact: false, value: "16px", padding: "12px", columns: 2 },
      "Summary styles do not leak into the popup",
    );
    await page
      .getByRole("button", { name: "Close calculation guide", exact: true })
      .click();
  };
  await checkCompact(page, true);
  const desktop = await browser.newPage({
    viewport: { width: 1200, height: 800 },
    reducedMotion: "reduce",
  });
  desktop.on("pageerror", (e) => errors.push(e.message));
  await desktop.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__facts`,
  );
  await checkCompact(desktop, false);
  await desktop.close();
  assert.deepEqual(requests, [], "Fact popups never read market data");
  assert.deepEqual(errors, []);
  console.log(
    "Preview fact browser passed: read-only floors, loading/empty/error states, preserved calculation and execution controls, and no market requests.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
