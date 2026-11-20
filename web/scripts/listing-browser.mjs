import { RangeWatch, orderResult } from "./range-fixture.mjs";
import { criteriaReport, giftReport } from "./valuation-fixture.mjs";
// Live transitions and valuation lifecycle against isolated React components and a fake feed.
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer } from "vite";
import { chromium } from "playwright";
const root = new URL("..", import.meta.url).pathname;
const html = `<!doctype html><html><body><div id="fixture"></div><script type="module">
import React,{useState,useEffect} from 'react';
import {createRoot} from 'react-dom/client';
import {useOrderFeed} from '/src/use-order-feed.ts';
import {GiftGrid} from '/src/orderbook/gift-grid.tsx';
import {GiftCard} from '/src/orderbook/gift-card.tsx';
import {Modal} from '/src/ui/modal.tsx';
import {OrderDetails} from '/src/orderbook/order-details.tsx';
import {orderKey} from '/src/orderbook/order-activity.ts';
import '/src/style.css';
const query={filters:{collection_id:'gift',currency_id:'GRAM',venue_id:null,models:[],backdrops:[],patterns:[],number:null,price:null},source:'asks',sort:'time_desc'};
const options=new Map([
['gold',{kind:'model',id:'gold',name:'Gold',media:{thumbnail_url:'/__gift.svg',animation_url:null}}],
['symbol',{kind:'pattern',id:'symbol',name:'Star',media:{thumbnail_url:'/__symbol.svg',animation_url:'/__symbol-motion'}}],
['blue',{kind:'backdrop',id:'blue',name:'Sapphire',palette:{center:'#759fb4',edge:'#364e73',pattern:'#283d60'}}]
]);
const names=new Map([['gift','Gift'],['gold','Gold'],['blue','Sapphire'],['symbol','Star'],['venue','MRKT']]);
function App(){const feed=useOrderFeed(query,false,0);const [selected,select]=useState();
window.selectListing=select;window.feedView=()=>feed;window.renameCollection=name=>names.set("gift",name);
useEffect(()=>{select(old=>old ? feed.displayOrders.find(row=>orderKey(row)===orderKey(old))||old:undefined)},[feed.displayOrders]);
return React.createElement('main',{className:'workspace'},React.createElement('div',{className:'orderbook'},
React.createElement('h2',{style:{margin:'14px 0'}},'Live gift market'),
React.createElement(GiftGrid,{orders:feed.displayOrders,busy:feed.loadingMore,visible:feed.setVisibleRange,card:order=>({options,collectionName:'Gift',venueName:'MRKT',selected:selected?.id===order.id,open:select,activity:feed.activity.get(orderKey(order))})})),
selected&&React.createElement(OrderDetails,{selected,options,names,collectionName:'Gift',collectionSlug:'Gift',removed:feed.activity.get(orderKey(selected))?.state==='removed',close:()=>select(undefined)}));}
const app=createRoot(document.getElementById('fixture'));app.render(React.createElement(App));window.unmount=()=>app.unmount();
window.renderListing=(order,activity)=>app.render(React.createElement('ul',{className:'gift-grid'},React.createElement(GiftCard,{order,activity,options,collectionName:'Gift',venueName:'MRKT',selected:false,open:()=>{}})));
window.renderDetails=order=>app.render(React.createElement(OrderDetails,{selected:order,options,names,collectionName:'Gift',collectionSlug:'Gift',close:()=>app.render(null)}));
window.renderMotionModal=()=>app.render(React.createElement(Modal,{label:'Motion test',heading:'Motion test',className:'valuation-popup',closeLabel:'Close motion test',close:()=>{window.modalCloseCalls=(window.modalCloseCalls||0)+1;app.render(null);}},React.createElement('p',null,'Motion fixture')));

</script></body></html>`;
const methodology = await readFile(
  new URL(
    "../../crates/market-valuation/src/gift_floor/methodology.md",
    import.meta.url,
  ),
  "utf8",
);
const cacheDir = await mkdtemp(join(tmpdir(), "market-listings-"));
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "listing-fixture",
      configureServer(server) {
        server.middlewares.use("/__listing_test", async (_, res, next) => {
          try {
            res.setHeader("Content-Type", "text/html");
            // Vite uses style elements in development; block style attributes as production does.
            res.setHeader(
              "Content-Security-Policy",
              "style-src-attr 'none'; style-src-elem 'self' 'unsafe-inline'",
            );
            res.end(await server.transformIndexHtml("/__listing_test", html));
          } catch (error) {
            next(error);
          }
        });
      },
    },
  ],
});
const order = (id, number) => ({
  id,
  book: { venue_id: "venue", collection_id: "gift", currency_id: "GRAM" },
  kind: "ask",
  target: {
    kind: "gift",
    id: `g${number}`,
    number,
    traits: { model: "gold", backdrop: "blue", pattern: "symbol" },
  },
  price: String(number * 10),
  quantity: 1,
  observed_at: "2026-11-18T12:00:00Z",
  executed_at: null,
  expires_at: null,
  market_url: "https://example.com/gift",
});
const a = order("a", 1),
  b = order("b", 2),
  x = order("x", 3);
let orderWire;
let watcher;
const frame = (rows) => watcher.frame(orderWire, rows);
function report(body) {
  const number = body.gift.number;
  const { kind, ...gift } = order("gift", number).target;
  const missing = {
    error: {
      code: "not_enough_data",
      message: "More backdrop listings are needed.",
      required_samples: 3,
      available_samples: 1,
    },
  };
  return {
    strategy: "fixture_floor",
    stale: false,
    evaluated_at: "2026-11-18T12:00:00Z",
    slug: null,
    gift: { ...gift, collection_id: "gift" },
    currency_id: body.currency_id,
    collection_floor: { value: "8" },
    model_floor: { value: String(number * 10) },
    backdrop_floor: { value: "9" },
    model_backdrop_floor: { value: "12" },
    model_multiplier: { value: "1.25" },
    backdrop_multiplier: missing,
    final_mark: missing,
    display: {
      fields: [
        {
          id: "confidence",
          label: "Fixture confidence",
          primary: false,
          value: { kind: "number", amount: "87.500", suffix: "%" },
        },
        {
          id: "decision",
          label: "Fixture decision",
          primary: true,
          value: {
            kind: "unavailable",
            label: "Not enough data",
            message: missing.error.message,
            detail: "1 of 3 fixture observations",
          },
        },
        {
          id: "model_floor",
          label: "Fixture price",
          primary: false,
          value: {
            kind: "currency",
            amount: String(number * 10),
            currency_id: body.currency_id,
          },
        },
        {
          id: "model_multiplier",
          label: "Fixture score",
          primary: false,
          value: { kind: "number", amount: "1.2500", suffix: " points" },
        },
      ],
      summary: "5 fixture observations",
      note: `A note owned by the fixture strategy. Gift #${number}.`,
    },
    evidence: {
      collection: { listings: 5, gifts: 5 },
      model: { listings: 3, gifts: 3 },
      backdrop: { listings: 1, gifts: 1 },
      model_backdrop: { listings: 1, gifts: 1 },
      minimum_multiplier_gifts: 3,
    },
  };
}
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 440, height: 880 } });
  const settled = () =>
    page.waitForFunction(
      () =>
        document
          .querySelector(".valuation-summary")
          ?.getAttribute("aria-busy") === "false",
    );
  const waitReport = async () => {
    await settled();
    await page.locator('.valuation-summary [data-metric="decision"]').waitFor();
  };
  const toggleGuide = async () => {
    const popup = page.locator(".valuation-popup");
    if (await popup.count()) {
      await page
        .getByRole("button", { name: "Close calculation guide", exact: true })
        .click();
      await popup.waitFor({ state: "detached" });
    } else {
      await page
        .getByRole("button", { name: "How is this calculated?", exact: true })
        .click();
      await popup.waitFor();
    }
  };
  const closeDetails = async () => {
    if (await page.locator(".valuation-popup").count()) await toggleGuide();
    await page
      .getByRole("button", { name: "Close details", exact: true })
      .click();
    await page.locator(".detail").waitFor({ state: "detached" });
  };
  let symbolAnimations = 0;
  page.on("request", (request) => {
    if (request.url().includes("/__symbol-motion")) symbolAnimations++;
  });
  const symbolPng = await page.evaluate(() => {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 96;
    const context = canvas.getContext("2d");
    context.scale(4, 4);
    context.fillStyle = "red";
    context.fill(new Path2D("m12 1 3 7 8 1-6 6 2 8-7-4-7 4 2-8-6-6 8-1z"));
    return canvas.toDataURL("image/png").split(",")[1];
  });
  await page.route("**/__symbol.svg", (route) =>
    route.fulfill({
      contentType: "image/png",
      body: Buffer.from(symbolPng, "base64"),
    }),
  );
  const errors = [];
  await page.addInitScript(() => {
    window.detailPatternDraws = [];
    const draw = CanvasRenderingContext2D.prototype.drawImage;
    CanvasRenderingContext2D.prototype.drawImage = function (source, ...args) {
      if (this.canvas.matches(".detail-art .gift-backdrop"))
        window.detailPatternDraws.push({
          size: args[2],
          opacity: this.globalAlpha,
        });
      return draw.call(this, source, ...args);
    };
    window.motionAnimations = [];
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (...args) {
      const animation = animate.apply(this, args);
      window.motionAnimations.push({ target: this, animation });
      return animation;
    };
  });
  let guideModuleRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/src/orderbook/valuation-methodology.tsx"))
      guideModuleRequests++;
  });
  page.on("pageerror", (error) => {
    errors.push(error.message);
    console.error(error);
  });
  await page.clock.install({ time: new Date("2026-11-18T12:00:00Z") });
  await page.route("**/__gift.svg", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: `<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128"><defs><linearGradient id="g" x2="1" y2="1"><stop stop-color="#fff0a2"/><stop offset="1" stop-color="#e4a536"/></linearGradient></defs><path d="M23 49h82v62H23z" fill="url(#g)"/><path d="M17 37h94v20H17z" fill="#f3d273"/><path d="M57 38h15v73H57z" fill="#fdf5cf"/><path d="M63 36c-38 0-31-33-11-18 8 5 11 18 11 18Zm2 0c38 0 31-33 11-18-8 5-11 18-11 18Z" fill="none" stroke="#ffefb0" stroke-width="8" stroke-linejoin="round"/></svg>`,
    }),
  );
  let metadataFails = true;
  let metadataRequests = 0,
    holdMetadata = false,
    releaseMetadata;
  const strategyMetadata = {
    id: "fixture_floor",
    input: "resolved_gift",
    targets: ["gift"],
    requires_period: false,
    name: "Fixture floor",
    description: "Discovered strategy description.",
    documentation_url: "/__fixture_strategy.md",
    documentation_revision: "r1",
  };
  await page.route("**/valuations/strategies", async (route) => {
    metadataRequests++;
    if (holdMetadata)
      await new Promise((resolve) => {
        releaseMetadata = resolve;
      });
    return route.fulfill(
      metadataFails
        ? { status: 503, json: { error: { message: "Unavailable" } } }
        : {
            json: [
              strategyMetadata,
              {
                ...strategyMetadata,
                id: "ask_floor",
                input: "selection",
                targets: ["criteria"],
              },
            ],
          },
    );
  });
  let guideRequests = 0;
  let guideFails = true,
    holdGuide = false,
    releaseGuide;
  let abortedGuides = 0,
    abortedMetadata = 0;
  page.on("requestfailed", (request) => {
    if (request.url().endsWith("/__fixture_strategy.md")) abortedGuides++;
    if (request.url().endsWith("/valuations/strategies")) abortedMetadata++;
  });
  await page.route("**/__fixture_strategy.md", async (route) => {
    guideRequests++;
    if (holdGuide)
      await new Promise((resolve) => {
        releaseGuide = resolve;
      });
    try {
      await route.fulfill(
        guideFails
          ? { status: 503, body: "Unavailable" }
          : {
              contentType: "text/markdown; charset=utf-8",
              body:
                methodology +
                `\n\nGuide revision ${strategyMetadata.documentation_revision}.\n` +
                "\n<script>window.guideExecuted=true</script>\n\n[Unsafe](javascript:alert(1))",
            },
      );
    } catch {
      /* A collapsed guide cancels its request. */
    }
  });
  let socket;
  let reportKind = "partial";
  let mode = "ok",
    release,
    requests = [];
  const cancelled = new Set();
  await page.route("**/valuations/query", () =>
    assert.fail("GUI valuation must use WebSocket"),
  );
  let connections = 0,
    activeConnections = 0;
  await page.routeWebSocket("**/ws", (peer) => {
    connections++;
    activeConnections++;
    peer.onClose(() => activeConnections--);
    socket = peer;
    peer.send(JSON.stringify({ type: "market.status", status: "available" }));
    peer.onMessage(async (message) => {
      const command = JSON.parse(String(message));
      if (command.type === "cancel") {
        cancelled.add(command.id);
        return;
      }
      if (command.type === "orders.watch") {
        orderWire = command.id;
        watcher = new RangeWatch(command.params, [a, b]);
        peer.send(JSON.stringify(frame([a, b])));
        return;
      }
      if (command.type !== "request") return;
      if (command.method === "orders.query") {
        peer.send(
          JSON.stringify({
            type: "result",
            id: command.id,
            method: command.method,
            ...orderResult(command.params, [a, b]),
          }),
        );
        return;
      }
      if (
        command.method === "valuation.query" &&
        command.params.target?.kind === "criteria"
      ) {
        peer.send(
          JSON.stringify({
            type: "result",
            id: command.id,
            method: command.method,
            data: criteriaReport(command.params),
          }),
        );
        return;
      }
      assert.equal(command.method, "valuation.query");
      const body = {
        gift: command.params.target.gift,
        currency_id: command.params.currency_id,
      };
      requests.push(body);
      if (mode === "hold")
        await new Promise((resolve) => {
          release = resolve;
        });
      const response = report(body);
      if (reportKind === "identity") response.gift.number++;
      if (reportKind !== "partial") {
        response.display.fields.find((field) => field.primary).value = {
          kind: "currency",
          amount: "25.00",
          currency_id: reportKind === "currency" ? "XTR" : body.currency_id,
        };
        if (reportKind === "stale") response.stale = true;
      }
      // Deliberately deliver late results too: canceled IDs must be ignored by the client.
      peer.send(
        JSON.stringify(
          mode === "error"
            ? {
                type: "error",
                id: command.id,
                error: { code: "unavailable", message: "Please try again" },
              }
            : {
                type: "result",
                id: command.id,
                method: command.method,
                data: giftReport(command.params, response),
              },
        ),
      );
    });
  });
  await page.goto(
    `http://127.0.0.1:${server.httpServer.address().port}/__listing_test`,
  );
  await page.locator(".gift-card").nth(1).waitFor();
  assert.equal(requests.length, 0);
  assert.equal(await page.locator(".activity-ribbon").count(), 0);
  assert.equal(
    await page
      .locator(".gift-card")
      .first()
      .evaluate((el) => getComputedStyle(el).animationName),
    "none",
    "Baseline listings do not animate into view",
  );
  socket.send(JSON.stringify(frame([x, a, b])));
  await page.locator('[data-order-id="x"]').waitFor();
  assert.equal(
    await page.locator(".is-new .activity-ribbon").textContent(),
    "New",
  );
  assert.equal(
    await page
      .locator(".is-new")
      .evaluate((el) => getComputedStyle(el).animationName),
    "listing-arrive",
  );
  assert.equal(
    await page
      .locator(".is-new")
      .evaluate((el) => getComputedStyle(el).animationDuration),
    "0.18s",
  );
  const clock = page.locator(".is-new .card-clock");
  assert.equal(
    await clock.evaluate((el) => getComputedStyle(el).animationDuration),
    "60s",
  );
  await clock.evaluate((el) => {
    el.getAnimations()[0].currentTime = 15000;
  });
  assert.ok(
    await clock.evaluate(
      (el) =>
        parseFloat(getComputedStyle(el).getPropertyValue("--clock-angle")) > 60,
    ),
  );
  await mkdir("/tmp/gifts-market-ui", { recursive: true });
  await page.screenshot({ path: "/tmp/gifts-market-ui/fresh-listings.png" });
  await page.locator('[data-order-id="a"]').click();
  await page
    .locator('.valuation-summary [role="alert"]')
    .getByText("Unavailable", { exact: true })
    .waitFor();
  assert.equal(
    requests.length,
    0,
    "Discovery failures do not dispatch guessed strategies",
  );
  metadataFails = false;
  await page
    .getByRole("button", { name: "Refresh valuation", exact: true })
    .click();
  await page.locator('.valuation-summary [data-metric="decision"]').waitFor();
  assert.ok(
    await page.evaluate(() =>
      window.motionAnimations.some(
        ({ target, animation }) =>
          target.classList.contains("detail") && animation.id === "modal-enter",
      ),
    ),
  );
  assert.equal(await page.locator(".price-comparison").count(), 0);
  assert.equal(await page.locator(".valuation-breakdown").count(), 0);
  assert.equal(
    await page.locator(".detail-attributes [data-metric]").count(),
    0,
  );
  const art = await page.locator(".detail-art").boundingBox();
  const giftArt = await page
    .locator(".detail-art .gift-model-image")
    .boundingBox();
  assert.ok(
    art.height >= 300 && giftArt.height > 200,
    "Mobile artwork has more room",
  );
  assert.equal(await page.locator(".detail-caption p").textContent(), "Gold");
  assert.equal(
    await page.locator(".detail-art #detail-title").textContent(),
    "Gift #1",
  );
  assert.equal(await page.locator(".detail-content #detail-title").count(), 0);
  await page.waitForFunction(() => {
    const tiles = window.detailPatternDraws.slice(-18);
    return (
      tiles.length === 18 &&
      new Set(tiles.map((tile) => tile.size)).size === 3 &&
      new Set(tiles.map((tile) => tile.opacity)).size > 3
    );
  });
  assert.ok(
    await page.locator(".detail-caption").evaluate((caption) => {
      const bounds = caption.getBoundingClientRect();
      const artwork = caption
        .closest(".detail-art")
        .querySelector(".gift-model-image")
        .getBoundingClientRect();
      return bounds.top >= artwork.bottom - 1;
    }),
    "The caption sits below the artwork",
  );
  assert.equal(await page.locator(".attribute-visual").count(), 3);
  await page.locator(".attribute-model .gift-model-image.loaded").waitFor();
  await page.locator(".attribute-pattern .gift-model-image.loaded").waitFor();
  await page.waitForFunction(
    () => document.querySelector(".attribute-backdrop canvas")?.width > 1,
  );
  assert.ok(
    (
      await page
        .locator(".attribute-pattern img")
        .evaluate((el) => getComputedStyle(el).filter)
    ).includes("brightness(0)"),
  );
  await page
    .locator(".detail")
    .evaluate((dialog) =>
      Promise.all(
        dialog.getAnimations().map((animation) => animation.finished),
      ),
    );
  await page.screenshot({ path: "/tmp/gifts-market-ui/details-mobile.png" });
  await page.evaluate(() => {
    window.detailNodes = [
      document.querySelector(".detail"),
      document.querySelector(".detail-art .gift-model-image"),
      document.querySelector(".valuation-summary"),
    ];
  });
  for (const width of [800, 959, 960, 1100, 1440]) {
    await page.setViewportSize({ width, height: 850 });
    const geometry = await page.locator(".detail").evaluate((dialog) => {
      const art = dialog.querySelector(".detail-art").getBoundingClientRect();
      const body = dialog.querySelector(".detail-body").getBoundingClientRect();
      const pane = dialog
        .querySelector(".detail-content")
        .getBoundingClientRect();
      const bounds = dialog.getBoundingClientRect();
      return {
        mobile:
          body.top >= art.bottom - 1 &&
          Math.abs(bounds.bottom - innerHeight) < 1,
        desktop:
          pane.left >= art.right - 1 &&
          Math.abs(pane.top - art.top) < 1 &&
          Math.abs(pane.height - art.height) < 1,
        closeVisible: (() => {
          const button = dialog.querySelector(".modal-close");
          const r = button.getBoundingClientRect();
          return (
            r.top >= 0 &&
            r.right <= innerWidth &&
            button.contains(
              document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2),
            )
          );
        })(),
        contained:
          bounds.left >= 0 &&
          bounds.right <= innerWidth &&
          bounds.top >= 0 &&
          bounds.bottom <= innerHeight &&
          dialog.scrollWidth <= dialog.clientWidth,
      };
    });
    assert.ok(geometry.contained, `Details fit the viewport at ${width}px`);
    assert.ok(
      geometry.closeVisible,
      `The close action is reachable at ${width}px`,
    );
    assert.ok(
      width < 960 ? geometry.mobile : geometry.desktop,
      `Responsive detail layout at ${width}px`,
    );
    assert.equal(
      await page.evaluate(
        () =>
          document.querySelector(".detail") === window.detailNodes[0] &&
          document.querySelector(".detail-art .gift-model-image") ===
            window.detailNodes[1] &&
          document.querySelector(".valuation-summary") ===
            window.detailNodes[2],
      ),
      true,
      "Resizing keeps the dialog, artwork and valuation mounted",
    );
  }
  await page.screenshot({ path: "/tmp/gifts-market-ui/details-desktop.png" });
  await page.setViewportSize({ width: 1024, height: 400 });
  const fixedDetails = await page.evaluate(() => ({
    art: document.querySelector(".detail-art").getBoundingClientRect().top,
    close: document
      .querySelector(".detail > .modal-close")
      .getBoundingClientRect().top,
    page: scrollY,
  }));
  const pane = page.locator(".detail-content");
  await pane.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  assert.ok(
    await pane.evaluate((element) => element.scrollTop > 0),
    "Short desktop windows scroll the information pane",
  );
  assert.deepEqual(
    await page.evaluate(() => ({
      art: document.querySelector(".detail-art").getBoundingClientRect().top,
      close: document
        .querySelector(".detail > .modal-close")
        .getBoundingClientRect().top,
      page: scrollY,
    })),
    fixedDetails,
    "Artwork, close button and listings stay still while details scroll",
  );
  assert.ok(
    await page.locator(".detail-actions").evaluate((element) => {
      const action = element.getBoundingClientRect();
      const dialog = element.closest(".detail").getBoundingClientRect();
      return action.top >= dialog.top && action.bottom <= dialog.bottom;
    }),
    "The venue action stays inside the short dialog",
  );
  await page.setViewportSize({ width: 440, height: 880 });
  await waitReport();
  assert.equal(
    symbolAnimations,
    0,
    "Attribute thumbnails never request animations",
  );
  assert.equal(requests.length, 1);
  assert.equal(guideRequests, 0, "Guide is not requested while collapsed");
  assert.deepEqual(requests[0], {
    gift: { collection_id: "gift", number: 1 },
    currency_id: "GRAM",
  });
  assert.equal(await page.locator(".valuation-metrics > li").count(), 1);
  assert.equal(
    await page.locator(".gift-attributes [data-metric]").count(),
    0,
    "Attribute previews never fall back to gift-strategy fields when catalog values are absent",
  );
  assert.equal(
    await page.getByText("5 fixture observations", { exact: true }).count(),
    0,
  );
  assert.equal(await page.locator(".valuation-summary time").count(), 0);
  assert.equal(
    await page
      .getByText("A note owned by the fixture strategy.", { exact: false })
      .count(),
    0,
  );
  await page
    .locator(".valuation-summary")
    .getByText("Not enough data", { exact: true })
    .first()
    .waitFor();
  for (const label of [
    "Listing information",
    "Last observed",
    "Venue",
    "Expires",
    "Order ID",
    "Gift ID",
  ])
    assert.equal(
      await page.locator(".detail").getByText(label, { exact: true }).count(),
      0,
    );
  socket.send(JSON.stringify(frame([x, b])));
  await page.locator(".is-removed").waitFor();
  assert.equal(
    await page
      .locator(".is-removed .card-preview")
      .evaluate((el) => getComputedStyle(el).transitionDuration),
    "0.25s, 0.25s",
  );
  assert.equal(
    await page.locator(".is-removed .activity-ribbon").textContent(),
    "Gone",
  );
  await page.waitForFunction(
    () =>
      getComputedStyle(document.querySelector(".is-removed .card-preview"))
        .filter === "grayscale(1)",
  );
  await page.locator(".listing-gone").waitFor();
  assert.equal(await page.locator(".detail-market").count(), 0);
  assert.equal(requests.length, 1, "Feed changes do not poll valuations");
  await page.screenshot({ path: "/tmp/gifts-market-ui/valuation-preview.png" });
  await closeDetails();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.waitForFunction(() => !document.querySelector(".card-clock"));
  assert.equal(await page.locator(".card-clock").count(), 0);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.clock.runFor(61000);
  await page.waitForFunction(() => !document.querySelector(".activity-ribbon"));
  assert.equal(await page.locator('[data-order-id="a"]').count(), 0);
  assert.equal(requests.length, 1);
  // A slow response must not populate the next gift's panel, even without unmounting.
  mode = "hold";
  socket.send(JSON.stringify(frame([a, b])));
  await page.locator('[data-order-id="a"]').waitFor();
  await page.locator('[data-order-id="a"]').click();
  await page.waitForFunction(
    () =>
      document
        .querySelector(".valuation-summary")
        ?.getAttribute("aria-busy") === "true",
  );
  for (let tries = 0; !release && tries < 100; tries++)
    await new Promise((resolve) => setTimeout(resolve, 10));
  assert.ok(release, "The delayed valuation request started");
  mode = "ok";
  await page.evaluate((order) => window.selectListing(order), b);
  await waitReport();
  release();
  assert.ok((await page.locator("#detail-title").textContent()).includes("#2"));
  mode = "error";
  await page
    .getByRole("button", { name: "Refresh valuation", exact: true })
    .click();
  await page
    .locator('.valuation-summary [role="alert"]')
    .getByText("Please try again", { exact: true })
    .waitFor();
  mode = "ok";
  await page
    .getByRole("button", { name: "Refresh valuation", exact: true })
    .click();
  await waitReport();
  assert.equal(
    guideModuleRequests,
    0,
    "The Markdown and math renderer stay unloaded until the guide opens",
  );
  const guide = page.locator(".valuation-popup");
  const info = page.getByRole("button", {
    name: "How is this calculated?",
    exact: true,
  });
  const metadataBeforePopup = metadataRequests;
  assert.equal(guideRequests, 0, "Guides remain intent-only");
  await info.focus();
  await page.keyboard.press("Enter");
  await page
    .locator(".valuation-popup .modal-heading")
    .getByRole("heading", { name: strategyMetadata.name, exact: true })
    .waitFor();
  assert.equal(
    metadataRequests,
    metadataBeforePopup,
    "The popup uses the request's discovered metadata",
  );
  assert.ok(
    (await guide.textContent()).includes("Gift #2."),
    "A delayed estimate cannot replace the next gift's result",
  );
  assert.equal(
    await guide.locator("time").count(),
    1,
    "Calculation timestamps stay in the info popup",
  );
  await guide.getByRole("button", { name: "Retry guide" }).waitFor();
  guideFails = false;
  await guide.getByRole("button", { name: "Retry guide" }).click();
  const content = guide.getByRole("article", {
    name: `${strategyMetadata.name} calculation guide`,
  });
  await content
    .getByRole("heading", { name: "Gift floor valuation", exact: true })
    .waitFor();
  assert.equal(guideRequests, 2, "Guide retries after an error");
  assert.deepEqual(
    await guide.locator(".valuation-metrics > li h4").allTextContents(),
    [
      "Fixture confidence",
      "Fixture decision",
      "Fixture price",
      "Fixture score",
    ],
  );
  assert.equal(
    await guide
      .locator('[data-metric="confidence"] .valuation-value')
      .textContent(),
    "87.500%",
  );
  assert.equal(
    await guide
      .locator('[data-metric="model_multiplier"] .valuation-value')
      .textContent(),
    "1.2500 points",
  );
  await guide.getByText("5 fixture observations", { exact: true }).waitFor();
  assert.equal(await page.locator(".gift-attributes [data-metric]").count(), 0);

  assert.ok(
    guideModuleRequests > 0,
    "The renderer loads only after discovery and expansion",
  );
  assert.ok(
    (await content.locator(".katex").count()) >= 3,
    "Inline and block LaTeX formulas render",
  );
  assert.ok(
    (await content.locator("math").count()) >= 3,
    "MathML remains available for accessibility",
  );
  assert.equal(
    await content.locator("[style]").count(),
    0,
    "Native MathML needs no inline styles under the production content policy",
  );
  assert.equal(
    await content.locator(".katex-error").count(),
    0,
    "The strategy guide has valid formulas",
  );
  await page.evaluate(() => document.fonts.ready);
  assert.ok((await content.textContent()).includes("midpoint-to-even"));
  assert.equal(await content.locator("script").count(), 0);
  assert.equal(await page.evaluate(() => window.guideExecuted), undefined);
  assert.equal(
    await content
      .getByRole("link", { name: "Unsafe", exact: true })
      .getAttribute("href"),
    "",
  );
  await page.keyboard.press("Tab");
  assert.equal(
    await guide.evaluate((el) => el.contains(document.activeElement)),
    true,
    "Keyboard focus stays in the popup",
  );
  await page.mouse.click(2, 2);
  await guide.waitFor({ state: "detached" });
  assert.equal(await guide.count(), 0, "Backdrop click closes only the guide");
  assert.equal(await page.locator(".detail").count(), 1);
  assert.equal(
    await info.evaluate((el) => el === document.activeElement),
    true,
  );
  await toggleGuide();
  await content.waitFor();
  await toggleGuide();
  await toggleGuide();
  assert.equal(
    guideRequests,
    2,
    "Collapsing and reopening reuses cached rendered math",
  );
  // Refreshes keep the breakdown, metadata and already-rendered guide alive.
  const metadataBeforeRefresh = metadataRequests;
  const guidesBeforeRefresh = guideRequests;
  assert.ok(cancelled.size > 0, "Closing details cancels the wire request");
  const requestsBeforeRefresh = requests.length;
  await content.evaluate((node) => {
    window.__guideBeforeRefresh = node.querySelector("math");
  });
  mode = "hold";
  release = undefined;
  await page.evaluate(() =>
    document.querySelector('button[aria-label="Refresh valuation"]').click(),
  );
  await guide.getByText("Refreshing estimate…", { exact: true }).waitFor();
  for (let tries = 0; !release && tries < 100; tries++)
    await new Promise((resolve) => setTimeout(resolve, 10));
  assert.ok(release);
  assert.equal(await page.locator(".valuation-breakdown").count(), 0);
  assert.equal(
    await content.evaluate(
      (node) => window.__guideBeforeRefresh === node.querySelector("math"),
    ),
    true,
  );
  mode = "ok";
  release();
  await settled();
  assert.equal(requests.length, requestsBeforeRefresh + 1);
  assert.equal(
    metadataRequests,
    metadataBeforeRefresh,
    "Refreshing does not rediscover unchanged strategy metadata",
  );
  assert.equal(
    guideRequests,
    guidesBeforeRefresh,
    "Refreshing does not download the guide again",
  );
  assert.equal(
    await content.evaluate(
      (node) => window.__guideBeforeRefresh === node.querySelector("math"),
    ),
    true,
    "Refreshing does not typeset the guide again",
  );
  mode = "error";
  await page.evaluate(() =>
    document.querySelector('button[aria-label="Refresh valuation"]').click(),
  );
  await guide.getByText("Previous estimate shown.", { exact: true }).waitFor();
  assert.equal(
    await content.evaluate(
      (node) => window.__guideBeforeRefresh === node.querySelector("math"),
    ),
    true,
  );
  assert.equal(await page.locator(".price-comparison").count(), 0);
  mode = "ok";
  await page.evaluate(() =>
    document.querySelector('button[aria-label="Refresh valuation"]').click(),
  );
  await settled();
  reportKind = "stale";
  await page.evaluate(() =>
    document.querySelector('button[aria-label="Refresh valuation"]').click(),
  );
  await guide.getByText("Estimate may be outdated", { exact: true }).waitFor();
  reportKind = "partial";
  await page.evaluate(() =>
    document.querySelector('button[aria-label="Refresh valuation"]').click(),
  );
  await settled();
  await page.setViewportSize({ width: 390, height: 844 });
  await content
    .getByRole("heading", { name: "Final mark", exact: true })
    .scrollIntoViewIfNeeded();
  assert.equal(
    await page
      .locator(".valuation-popup")
      .evaluate((el) => el.scrollWidth > el.clientWidth),
    false,
    "Guide formulas fit mobile details",
  );
  await page.screenshot({
    path: "/tmp/gifts-market-ui/valuation-methodology.png",
  });
  const cachedMetadataRequests = metadataRequests;
  const cachedGuideRequests = guideRequests;
  await closeDetails();
  await page.evaluate((order) => window.selectListing(order), b);
  await waitReport();
  await toggleGuide();
  await page
    .getByRole("article", {
      name: `${strategyMetadata.name} calculation guide`,
    })
    .waitFor();
  assert.equal(
    metadataRequests,
    cachedMetadataRequests,
    "Reopening details reuses fresh metadata",
  );
  assert.equal(
    guideRequests,
    cachedGuideRequests,
    "Reopening details reuses rendered math without a new download",
  );
  await closeDetails();
  strategyMetadata.documentation_revision = "r2";
  strategyMetadata.name = "Updated fixture floor";
  await page.clock.runFor(60_001);
  const abortedGuidesBefore = abortedGuides;
  const abortedMetadataBefore = abortedMetadata;
  holdGuide = true;
  holdMetadata = true;
  releaseMetadata = undefined;
  await page.evaluate((order) => window.selectListing(order), b);
  for (let tries = 0; !releaseMetadata && tries < 100; tries++)
    await new Promise((resolve) => setTimeout(resolve, 10));
  assert.ok(releaseMetadata);
  assert.equal(
    guideRequests,
    cachedGuideRequests,
    "Guide reads wait for intent and current metadata",
  );
  await closeDetails();
  for (
    let tries = 0;
    abortedMetadata === abortedMetadataBefore && tries < 100;
    tries++
  )
    await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(
    abortedMetadata,
    abortedMetadataBefore + 1,
    "Closing details cancels pending discovery",
  );
  holdMetadata = false;
  releaseMetadata();
  await page.evaluate((order) => window.selectListing(order), b);
  await waitReport();
  await toggleGuide();
  await page
    .locator(".valuation-popup .modal-heading")
    .getByRole("heading", { name: strategyMetadata.name, exact: true })
    .waitFor();
  for (let tries = 0; !releaseGuide && tries < 100; tries++)
    await new Promise((resolve) => setTimeout(resolve, 10));
  assert.ok(releaseGuide);
  await toggleGuide();
  for (
    let tries = 0;
    abortedGuides === abortedGuidesBefore && tries < 100;
    tries++
  )
    await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(
    abortedGuides,
    abortedGuidesBefore + 1,
    "Collapsing a pending guide cancels its fetch",
  );
  assert.equal(await page.locator(".valuation-popup article").count(), 0);
  holdGuide = false;
  releaseGuide();
  await toggleGuide();
  await page
    .getByRole("article", {
      name: `${strategyMetadata.name} calculation guide`,
    })
    .getByText("Guide revision r2.", { exact: true })
    .waitFor();
  assert.equal(
    metadataRequests,
    cachedMetadataRequests + 2,
    "Expired metadata retries when cancelled details reopen",
  );
  assert.equal(
    guideRequests,
    cachedGuideRequests + 2,
    "New documentation revision loads after a cancelled attempt",
  );
  await page.keyboard.press("Escape");
  await guide.waitFor({ state: "detached" });
  assert.equal(await guide.count(), 0, "Escape closes only the info popup");
  assert.equal(await page.locator(".detail").count(), 1);
  assert.equal(
    await info.evaluate((el) => el === document.activeElement),
    true,
    "Focus returns to the info button",
  );
  // Only a single live currency estimate can be compared with an active ask.
  reportKind = "estimate";
  await page
    .getByRole("button", { name: "Refresh valuation", exact: true })
    .click();
  await page.getByText("About 20% below estimate", { exact: true }).waitFor();

  // A response started before an interruption stays outdated even if it arrives after recovery.
  mode = "hold";
  release = undefined;
  await page
    .getByRole("button", { name: "Refresh valuation", exact: true })
    .click();
  await page.waitForFunction(
    () =>
      document
        .querySelector(".valuation-summary")
        ?.getAttribute("aria-busy") === "true",
  );
  assert.ok(release);
  const pendingRequests = requests.length;
  socket.send(JSON.stringify({ type: "market.status", status: "delayed" }));
  await page.getByText("Estimate may be outdated", { exact: true }).waitFor();
  socket.send(JSON.stringify({ type: "market.status", status: "available" }));
  mode = "success";
  release();
  await settled();
  assert.equal(await page.locator(".price-comparison").count(), 0);
  assert.equal(
    requests.length,
    pendingRequests,
    "recovery must not duplicate in-flight requests",
  );
  await page.getByText("Estimate may be outdated", { exact: true }).waitFor();
  await page
    .getByRole("button", { name: "Refresh valuation", exact: true })
    .click();
  await page.getByText("About 20% below estimate", { exact: true }).waitFor();
  assert.equal(await page.locator(".valuation-breakdown").count(), 0);
  const requestsBeforeDelay = requests.length;
  socket.send(JSON.stringify({ type: "market.status", status: "delayed" }));
  await page.getByText("Estimate may be outdated", { exact: true }).waitFor();
  assert.equal(await page.locator(".price-comparison").count(), 0);
  socket.send(JSON.stringify({ type: "market.status", status: "available" }));
  await page.waitForTimeout(50);
  assert.equal(
    await page.locator(".price-comparison").count(),
    0,
    "recovery must not mark an old estimate current",
  );
  assert.equal(
    requests.length,
    requestsBeforeDelay,
    "availability changes must not trigger valuation requests",
  );
  await page
    .getByRole("button", { name: "Refresh valuation", exact: true })
    .click();
  await page.getByText("About 20% below estimate", { exact: true }).waitFor();

  await page.setViewportSize({ width: 320, height: 720 });
  assert.equal(
    await page
      .locator(".detail")
      .evaluate((el) => el.scrollWidth > el.clientWidth),
    false,
  );
  await page.screenshot({ path: "/tmp/gifts-market-ui/details-estimate.png" });
  for (const variant of ["stale", "currency"]) {
    reportKind = variant;
    await page
      .getByRole("button", { name: "Refresh valuation", exact: true })
      .click();
    await settled();
    await page
      .locator('.valuation-summary [data-metric="decision"] .valuation-value')
      .waitFor();
    if (variant === "stale")
      await page
        .getByText("Estimate may be outdated", { exact: true })
        .waitFor();
    else
      await page.locator(".valuation-summary .currency-symbol.star").waitFor();
    assert.equal(await page.locator(".price-comparison").count(), 0);
  }
  reportKind = "identity";
  await page
    .getByRole("button", { name: "Refresh valuation", exact: true })
    .click();
  await page
    .getByText("Gift details changed. Refresh this market and try again.", {
      exact: true,
    })
    .waitFor();
  assert.equal(await page.locator(".price-comparison").count(), 0);
  reportKind = "estimate";
  for (const kind of ["bid", "executed"]) {
    await page.evaluate((order) => window.selectListing(order), { ...b, kind });
    await page
      .getByRole("button", { name: "Refresh valuation", exact: true })
      .click();
    await settled();
    await page
      .locator('.valuation-summary [data-metric="decision"] .valuation-value')
      .waitFor();
    assert.equal(await page.locator(".price-comparison").count(), 0);
    await page
      .getByRole("heading", {
        name: kind === "bid" ? "Bid price" : "Execution price",
        exact: true,
      })
      .waitFor();
  }
  // Criteria use one matching-ask request, without invoking the individual gift estimate.
  const countBeforeCriteria = requests.length;
  await page.evaluate((order) => window.selectListing(order), {
    ...b,
    kind: "bid",
    target: {
      kind: "criteria",
      models: [],
      backdrops: [],
      patterns: [],
      number: null,
    },
  });
  await page.locator(".detail-criteria").waitFor();
  assert.equal(await page.locator(".detail .valuation-summary").count(), 0);
  assert.equal(await page.locator(".attribute-visual").count(), 0);
  assert.equal(await page.locator(".detail .condition-any").count(), 3);
  assert.equal(requests.length, countBeforeCriteria);
  assert.equal(
    await page.locator(".detail-caption p").count(),
    0,
    "Unrestricted bids do not invent a model name",
  );
  assert.equal(
    await page.locator("#detail-title span").count(),
    0,
    "Unnumbered bids have no gift number",
  );
  assert.equal(symbolAnimations, 0);
  await page.locator(".matching-floor .valuation-value").waitFor();
  // Standalone criteria details retain status while the matching-ask result is shown.
  const connectionsBeforeCriteria = connections;
  await page.evaluate(
    (order) =>
      window.renderDetails({
        ...order,
        kind: "bid",
        target: {
          kind: "criteria",
          models: [],
          backdrops: [],
          patterns: [],
          number: null,
        },
      }),
    b,
  );
  await page.locator(".detail-criteria").waitFor();
  assert.equal(await page.locator(".detail .valuation-summary").count(), 0);
  await page.waitForTimeout(500);
  assert.ok(connections >= connectionsBeforeCriteria);
  assert.equal(
    activeConnections,
    1,
    "criteria details retain the availability subscription until closed",
  );
  assert.equal(requests.length, countBeforeCriteria);
  // Every order kind shares the same bounded layout, including long collection names.
  await page.evaluate(() =>
    window.renameCollection(
      "An exceptionally long collectible collection name",
    ),
  );
  for (const kind of ["ask", "bid", "executed"]) {
    await page.evaluate((order) => window.renderDetails(order), { ...b, kind });
    for (const [width, height] of [
      [320, 568],
      [390, 844],
      [960, 600],
      [1200, 900],
      [1024, 400],
    ]) {
      await page.setViewportSize({ width, height });
      const layout = await page.locator(".detail").evaluate((dialog) => {
        dialog.scrollTop = 0;
        dialog.querySelector(".detail-content").scrollTop = 0;
        const bounds = dialog.getBoundingClientRect();
        const art = dialog.querySelector(".detail-art").getBoundingClientRect();
        const caption = dialog
          .querySelector(".detail-caption")
          .getBoundingClientRect();
        const media = dialog
          .querySelector(".detail-art .gift-model-image")
          .getBoundingClientRect();
        const close = dialog.querySelector(".modal-close");
        const button = close.getBoundingClientRect();
        const links = [...dialog.querySelectorAll(".detail-actions a")].map(
          (a) => a.getBoundingClientRect(),
        );
        return {
          contained:
            bounds.left >= 0 &&
            bounds.right <= innerWidth &&
            bounds.top >= 0 &&
            bounds.bottom <= innerHeight &&
            dialog.scrollWidth <= dialog.clientWidth,
          caption:
            caption.top >= media.bottom - 1 && caption.bottom <= art.bottom + 1,
          close: close.contains(
            document.elementFromPoint(
              button.x + button.width / 2,
              button.y + button.height / 2,
            ),
          ),
          oneActionRow:
            links.length === 2 && Math.abs(links[0].top - links[1].top) < 1,
          height: bounds.height,
        };
      });
      assert.ok(
        layout.contained &&
          layout.caption &&
          layout.close &&
          layout.oneActionRow,
        `${kind} at ${width}×${height}: ${JSON.stringify(layout)}`,
      );
      if (width === 1200)
        assert.ok(
          layout.height < 680,
          "Short content does not retain a fixed-height desktop gap",
        );
      if (width === 390 || width === 1200)
        await page.screenshot({
          path: `/tmp/gifts-market-ui/review-${kind}-${width}.png`,
        });
    }
  }
  await page.evaluate(
    (order) =>
      window.renderDetails({ ...order, market_url: "https://t.me/nft/Gift-2" }),
    b,
  );
  assert.equal(
    await page.locator(".detail-actions a").count(),
    1,
    "Identical venue and Telegram destinations share one action",
  );
  assert.equal(
    await page
      .getByRole("link", { name: "Open in Telegram", exact: true })
      .count(),
    1,
  );
  await page.evaluate(() => window.renameCollection("Gift"));
  // Retained activity timestamps prevent entry replay after virtualized remounts.
  await page.evaluate(
    (order) =>
      window.renderListing(order, { state: "new", since: Date.now() - 5000 }),
    a,
  );
  await page.locator(".is-new").waitFor();
  assert.equal(
    await page
      .locator(".is-new")
      .evaluate((el) =>
        el
          .getAnimations()
          .some((animation) => animation.animationName === "listing-arrive"),
      ),
    false,
  );
  assert.equal(
    await page
      .locator(".is-new")
      .evaluate((el) => getComputedStyle(el).opacity),
    "1",
  );
  await page.evaluate((order) => {
    window.motionAnimations = [];
    window.renderListing(order);
  }, a);
  await page.waitForFunction(() => !document.querySelector(".is-new"));
  await page.evaluate(
    (order) => window.renderListing({ ...order, price: "12" }),
    a,
  );
  await page.waitForFunction(
    () =>
      document.querySelector(".card-copy .quote-amount")?.textContent === "12",
  );
  assert.equal(
    await page.evaluate(
      () =>
        window.motionAnimations.filter(
          ({ animation }) => animation.id === "price-highlight",
        ).length,
    ),
    1,
    "Only changed prices flash",
  );
  await page.evaluate(
    (order) => window.renderListing({ ...order, price: "12", quantity: 2 }),
    a,
  );
  await page.locator(".card-head").waitFor();
  assert.equal(
    await page.evaluate(
      () =>
        window.motionAnimations.filter(
          ({ animation }) => animation.id === "price-highlight",
        ).length,
    ),
    1,
    "Unchanged prices do not restart the effect",
  );
  await page.evaluate(
    (order) => window.renderListing({ ...order, price: "13" }),
    a,
  );
  await page.waitForFunction(
    () =>
      document.querySelector(".card-copy .quote-amount")?.textContent === "13",
  );
  assert.equal(
    await page.evaluate(
      () =>
        window.motionAnimations.filter(
          ({ animation }) => animation.id === "price-highlight",
        ).length,
    ),
    2,
  );
  await page.evaluate(
    (order) =>
      window.renderListing({
        ...order,
        price: "14",
        book: { ...order.book, currency_id: "XTR" },
      }),
    a,
  );
  await page.locator(".card-copy .currency-symbol.star").waitFor();
  assert.equal(
    await page.evaluate(
      () =>
        window.motionAnimations.filter(
          ({ animation }) => animation.id === "price-highlight",
        ).length,
    ),
    2,
    "Currency switches do not flash",
  );
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.evaluate((order) => {
    window.motionAnimations = [];
    window.renderListing(
      { ...order, price: "15", book: { ...order.book, currency_id: "XTR" } },
      { state: "new", since: Date.now() },
    );
  }, a);
  await page.waitForFunction(
    () =>
      document.querySelector(".card-copy .quote-amount")?.textContent === "15",
  );
  assert.equal(
    await page
      .locator(".gift-card")
      .evaluate((el) => getComputedStyle(el).animationName),
    "none",
  );
  assert.equal(
    await page
      .locator(".card-preview")
      .evaluate((el) => getComputedStyle(el).transitionDuration),
    "0s",
  );
  assert.equal(
    await page.evaluate(() => window.motionAnimations.length),
    0,
    "Reduced motion skips price flashes",
  );
  await page.evaluate(() => {
    window.modalCloseCalls = 0;
    window.renderMotionModal();
  });
  await page
    .getByRole("dialog", { name: "Motion test", exact: true })
    .waitFor();
  assert.equal(
    await page.evaluate(() => window.motionAnimations.length),
    0,
    "Reduced motion skips modal entry",
  );
  await page
    .getByRole("button", { name: "Close motion test", exact: true })
    .click();
  await page.locator(".valuation-popup").waitFor({ state: "detached" });
  assert.equal(await page.evaluate(() => window.modalCloseCalls), 1);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.evaluate(() => {
    window.modalCloseCalls = 0;
    window.renderMotionModal();
  });
  const motionModal = page.getByRole("dialog", {
    name: "Motion test",
    exact: true,
  });
  await motionModal.waitFor();
  assert.ok(
    await page.evaluate(() =>
      window.motionAnimations.some(
        ({ animation }) => animation.id === "modal-enter",
      ),
    ),
  );
  await page.keyboard.press("Escape");
  await page.waitForFunction(
    () =>
      document.querySelector(".valuation-popup")?.dataset.closing === "true",
  );
  await page.keyboard.press("Escape");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await motionModal.waitFor({ state: "detached" });
  assert.equal(
    await page.evaluate(() => window.modalCloseCalls),
    1,
    "Motion preference changes and repeated Escape finish dismissal once",
  );
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.evaluate(() => {
    window.modalCloseCalls = 0;
    window.renderMotionModal();
  });
  await motionModal.waitFor();
  await page.evaluate((order) => {
    document
      .querySelector(".valuation-popup")
      .dispatchEvent(new Event("cancel", { cancelable: true }));
    window.renderListing(order);
  }, a);
  await motionModal.waitFor({ state: "detached" });
  await page.waitForTimeout(180);
  assert.equal(
    await page.evaluate(() => window.modalCloseCalls),
    0,
    "Unmount cancels an outstanding exit callback",
  );
  await page.evaluate(() => window.unmount());
  assert.deepEqual(errors, []);
  console.log(
    "Listing browser passed: 60-second clock, new/grey ribbons, bounded expiry, unified report, exact units, cancelled requests, retry, strategy discovery and lazy Markdown/LaTeX calculation guide.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
