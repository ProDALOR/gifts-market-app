// Actual shared pricing component: no query services, synthetic reports and static icons.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const cacheDir = await mkdtemp(join(tmpdir(), "igifts-pricing-"));
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module">
import React from'react';import{createRoot}from'react-dom/client';import{flushSync}from'react-dom';import{GiftPricing}from'/src/previews/gift-pricing.tsx';import'/src/style.css';
const h=React.createElement,root=createRoot(document.getElementById('root'));
window.show=(kind,price,currency,scale=1,long=false)=>{
 const valuations=['floor','bid'].map((id,index)=>{const field={id:'price',label:'Price',primary:true,value:{kind:'currency',amount:long?'123456789012345678901234567890.123456789':'8.33',currency_id:currency}};return{strategy:{id,name:long?'A long registered strategy name that must remain readable':index?'Best bid':'Gift floor',purpose:index?'reference':'estimate',description:'Calculation details',revision:'one',documentation_url:'/__guide',documentation_revision:'one',display:{note:'',fields:[],evidence:[]}},primaryFields:[field],fields:[field],evidence:[],stale:false}});
 const state={key:'gift',available:true,loading:false,valuations,report:{evaluated_at:'2026-11-26T12:00:00Z',results:[]}};
 const order=kind==='portfolio'?undefined:{kind,price,executed_at:kind==='executed'?'2026-11-26T12:00:00Z':null};
 flushSync(()=>root.render(h('main',{className:'detail-body pricing-test'},h(GiftPricing,{order,state,currency,criteria:kind==='bid'}))));
 const elements=[...document.querySelectorAll('.detail-pricing *')];for(const el of elements)el.style.removeProperty('font-size');
 const sizes=elements.map(el=>[el,parseFloat(getComputedStyle(el).fontSize)]);
 for(const[el,size]of sizes)el.style.fontSize=size*scale+'px';
};
window.show('ask','11','GRAM');
</script><style>.pricing-test{max-width:480px;margin:auto}.pricing-test .detail-pricing{margin:0}</style></body></html>`;
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "pricing-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url === "/__pricing") {
            res.setHeader("Content-Type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
            return;
          }
          if (req.url === "/__guide") {
            res.setHeader("Content-Type", "text/markdown");
            res.end("# Calculation\nExact observed market values.");
            return;
          }
          if (req.url.startsWith("/media/icons/")) {
            res.setHeader("Content-Type", "image/svg+xml");
            res.end(
              '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/></svg>',
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
  for (const ratio of [1, 1.25, 2, 3]) {
    const page = await browser.newPage({
        viewport: { width: 390, height: 900 },
        deviceScaleFactor: ratio,
        reducedMotion: "reduce",
      }),
      errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto(server.resolvedUrls.local[0] + "__pricing");
    for (const width of [280, 320, 390, 768, 960, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      for (const kind of ["ask", "bid", "executed", "portfolio"]) {
        for (const [price, currency, scale, long] of [
          ["11", "GRAM", 1, false],
          ["123456789012345678901234567890.123456789", "XTR", 1, false],
          ["123456789.1234567890123456789", "GRAM", 1.5, true],
        ]) {
          await page.evaluate(
            (args) => window.show(...args),
            [kind, price, currency, scale, long],
          );
          const layout = await page
            .locator(".detail-pricing")
            .evaluate((block) => {
              const box = block.getBoundingClientRect();
              const amounts = [...block.querySelectorAll(".quote-amount")];
              return {
                contained:
                  block.scrollWidth <= block.clientWidth &&
                  document.documentElement.scrollWidth <= innerWidth,
                fullText: amounts.every((el) => {
                  const r = document.createRange();
                  r.selectNodeContents(el);
                  return [...r.getClientRects()].every(
                    (rect) =>
                      rect.left >= box.left - 1 && rect.right <= box.right + 1,
                  );
                }),
                noScroll: amounts.every(
                  (el) =>
                    !["auto", "scroll"].includes(
                      getComputedStyle(el).overflowX,
                    ) &&
                    !["auto", "scroll"].includes(
                      getComputedStyle(el).overflowY,
                    ),
                ),
                main: block
                  .querySelector(".detail-price .quote-amount")
                  ?.textContent.replaceAll("\u00a0", ""),
              };
            });
          assert(
            layout.contained && layout.fullText && layout.noScroll,
            `${ratio}x ${width}px ${kind} ${scale}× font: ${JSON.stringify(layout)}`,
          );
          if (kind !== "portfolio")
            assert.equal(
              layout.main,
              price,
              "full financial precision stays visible",
            );
        }
      }
    }
    assert.deepEqual(errors, []);
    await page.close();
  }
  const page = await browser.newPage({
    viewport: { width: 390, height: 844 },
    reducedMotion: "reduce",
  });
  await page.goto(server.resolvedUrls.local[0] + "__pricing");
  await page
    .getByRole("button", { name: "How is Gift floor calculated?", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Calculation guide", exact: true })
    .waitFor();
  await page
    .getByRole("heading", { name: "Calculation", exact: true })
    .waitFor();
  assert.equal(
    await page
      .locator(".valuation-popup")
      .evaluate((el) => el.scrollWidth <= el.clientWidth),
    true,
    "calculation popup remains independently readable",
  );
  await page
    .getByRole("button", { name: "Close calculation guide", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Calculation guide", exact: true })
    .waitFor({ state: "detached" });
  await page.close();
  console.log(
    "Pricing browser passed: asks/bids/history/portfolio, 280–1440px, fractional/high DPI, large text, exact long amounts, percentage badges and calculation popup.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
