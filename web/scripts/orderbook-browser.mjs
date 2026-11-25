import { fixtureDisplay } from "./valuation-fixture.mjs";
// Full UI workflow with an isolated local catalog/feed; no Telegram or hub login.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { orderResult, RangeWatch } from "./range-fixture.mjs";
import {
  criteriaReport,
  floorReport,
  giftReport,
} from "./valuation-fixture.mjs";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const cacheDir = await mkdtemp(join(tmpdir(), "orderbook-"));
const server = await createServer({
  cacheDir,
  root: fileURLToPath(new URL("..", import.meta.url)),
  // This fixture explicitly exercises Mini App behavior, even with API-only schemas.
  define: { "import.meta.env.VITE_MARKET_MINIAPP": JSON.stringify("true") },
  server: { host: "127.0.0.1", port: 0 },
});
let browser;
const option = (id, name, kind, palette) => ({
  id,
  name,
  kind,
  ...(kind === "backdrop"
    ? { palette }
    : {
        media: {
          thumbnail_url: `/media/${kind === "pattern" ? "patterns" : "models"}/${id}/thumbnail`,
          animation_url:
            kind === "model" ? `/media/models/${id}/animation` : null,
        },
      }),
});
const models = [
  option("barbie", "Barbie", "model"),
  option("juicy", "Juicy Mint", "model"),
  option("grape", "Grape Glaze", "model"),
  option("ivory", "Ivory", "model"),
  option("rose", "Rose", "model"),
];
const backdrops = [
  option("raspberry", "Raspberry", "backdrop", {
    center: "#e07b85",
    edge: "#b65980",
    pattern: "#783950",
  }),
  option("hunter", "Hunter Green", "backdrop", {
    center: "#8fae78",
    edge: "#4b825b",
    pattern: "#285038",
  }),
];
backdrops.push(
  ...["Midnight", "Amber", "Lavender"].map((name, index) =>
    option(`backdrop-${index}`, name, "backdrop", {
      center: "#c493ab",
      edge: "#75506e",
      pattern: `#${["563447", "254829", "633553"][index]}`,
    }),
  ),
);
const patterns = [
  "Stars",
  "Heart",
  "Moon",
  "Clover",
  "Skull",
  "Sun",
  "Cloud",
  "Bolt",
].map((name) => option(name.toLowerCase(), name, "pattern"));
const collections = ["Jelly Bunny", "Plush Pepe"].map((name, index) => ({
  id: index ? "pepe" : "bunny",
  name,
  slug: null,
  preview: models[0].media,
  model_ids: models.map((o) => o.id),
  backdrop_ids: backdrops.map((o) => o.id),
  pattern_ids: patterns.map((o) => o.id),
}));
const venues = ["Telegram", "Portals", "MRKT"].map((name) => ({
  id: name.toLowerCase(),
  name,
}));
let historyExtras = [];
function result(query, executed = query.source === "history") {
  assert.ok(
    query.filters.collection_id,
    "Market never requests all collections",
  );
  const filters = {
    ...query.filters,
    ...Object.fromEntries(
      Object.entries({ models, backdrops, patterns }).map(([kind, options]) => [
        kind,
        query.filters[kind]?.map(
          (id) => options.find((row) => id === `@name:${row.name}`)?.id ?? id,
        ),
      ]),
    ),
  };
  const mode = executed ? "executed" : query.source === "bids" ? "bid" : "ask";
  let rows = Array.from(
    { length: executed ? 75 : mode === "bid" ? 4 : 125 },
    (_, index) => ({
      id: `order-${index}`,
      book: {
        collection_id:
          filters.collection_id || collections[index % collections.length].id,
        currency_id: filters.currency_id,
        venue_id: venues[index % 3].id,
      },
      kind: mode,
      quantity: mode === "bid" && index === 1 ? 5 : 1,
      price:
        filters.currency_id === "XTR"
          ? String(850 + index)
          : index === 0
            ? "0.500000001"
            : index === 59
              ? "10000000000000000.000000001"
              : String(index + 10),
      target:
        mode === "bid" && index === 0
          ? {
              kind: "unsupported",
              code: "unsupported_constraints",
              message:
                "This listing has conditions that cannot be displayed yet.",
            }
          : mode === "bid"
            ? {
                kind: "criteria",
                models:
                  index === 1
                    ? [models[0].id]
                    : index === 2
                      ? [models[0].id, models[1].id]
                      : [],
                backdrops: [],
                patterns: [],
                number: null,
              }
            : {
                kind: "gift",
                id: `gift-${1000 + index}`,
                number: 2000 - index,
                traits: {
                  model: models[index % models.length].id,
                  backdrop: backdrops[index % backdrops.length].id,
                  pattern: patterns[index % patterns.length].id,
                },
              },
      observed_at: new Date(1700000000000 + index * 1000).toISOString(),
      executed_at: executed
        ? new Date(1700000000000 + index * 1000).toISOString()
        : null,
      expires_at: null,
      market_url:
        mode === "bid" && index === 0
          ? null
          : "https://t.me/market/app?startapp=gift42",
    }),
  );
  if (executed) rows.push(...historyExtras);
  rows = rows.filter(
    (order) =>
      (!filters.venue_id || filters.venue_id === order.book.venue_id) &&
      ["models", "backdrops", "patterns"].every((key) =>
        mode === "bid" && query.bid_match === "all"
          ? order.target.kind === "criteria" &&
            order.target[key].length === (filters[key]?.length || 0) &&
            order.target[key].every((id) => filters[key]?.includes(id))
          : !filters[key]?.length ||
            (order.target.kind === "criteria"
              ? !order.target[key].length ||
                filters[key].some((id) => order.target[key].includes(id))
              : order.target.kind === "gift" &&
                filters[key].includes(
                  order.target.traits[
                    {
                      models: "model",
                      backdrops: "backdrop",
                      patterns: "pattern",
                    }[key]
                  ],
                )),
      ) &&
      (!filters.price?.max || Number(order.price) <= Number(filters.price.max)),
  );
  rows.sort((a, b) => {
    const rank = (order) =>
      query.sort?.startsWith("time_")
        ? Date.parse(executed ? order.executed_at : order.observed_at)
        : query.sort?.startsWith("number_")
          ? order.target.kind === "gift"
            ? order.target.number
            : 0
          : Number(order.price);
    return (rank(a) - rank(b)) * (query.sort?.endsWith("_desc") ? -1 : 1);
  });
  return { data: rows.slice(0, query.limit || 50) };
}
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({
    viewport: { width: 390, height: 844 },
    reducedMotion: "reduce",
  });
  await page.route("**/telegram", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        auth_enabled: false,
        bot_id: null,
        bot_username: null,
        public_url: null,
      }),
    }),
  );
  page.setDefaultTimeout(10000);
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const png = await page.evaluate(() => {
    const c = document.createElement("canvas");
    c.width = c.height = 16;
    const ctx = c.getContext("2d");
    ctx.fillStyle = "white";
    ctx.fillRect(2, 2, 12, 12);
    return c.toDataURL().split(",")[1];
  });
  await page.route("**/media/**", (route) =>
    route.fulfill({
      contentType: "image/png",
      body: Buffer.from(png, "base64"),
    }),
  );
  const directoryRevision = "RCdlHdPhcr-iTt64ylBkPA";
  const collectionRevisions = {
    bunny: "_y_lqMGEh6UieJB3_3_TTw",
    pepe: "EnnK_4Kr2qgEkLGpYM3NxA",
  };
  let releaseInitialCatalog;
  const initialCatalog = new Promise((resolve) => {
    releaseInitialCatalog = resolve;
  });
  let catalogRequests = 0;
  let failOptions = true,
    optionRequests = 0;
  await page.route("**/catalog", async (route) => {
    catalogRequests++;
    await initialCatalog;
    return route.fulfill({
      headers: { etag: `"${directoryRevision}"` },
      json: {
        collections,
        backdrops: [
          {
            ...backdrops[0],
            aliases: [{ namespace: "name", id: "Raspberry" }],
          },
        ],
        patterns: [
          { ...patterns[0], aliases: [{ namespace: "name", id: "Stars" }] },
        ],
        venues,
        currencies: [
          { id: "GRAM", name: "Gram", decimals: 9 },
          { id: "XTR", name: "Stars", decimals: 0 },
        ],
        books: collections.flatMap((item) =>
          ["GRAM", "XTR"].flatMap((currency) =>
            venues.map((venue) => ({
              collection_id: item.id,
              currency_id: currency,
              venue_id: venue.id,
            })),
          ),
        ),
      },
    });
  });
  await page.route(
    (url) => url.pathname.startsWith("/collections/"),
    (route) => {
      optionRequests++;
      if (failOptions) {
        failOptions = false;
        return route.fulfill({
          status: 503,
          json: {
            error: { code: "unavailable", message: "Choices unavailable" },
          },
        });
      }
      const id = decodeURIComponent(
        new URL(route.request().url()).pathname.split("/").at(-1),
      );
      return route.fulfill({
        headers: { etag: `"${collectionRevisions[id]}"` },
        json: {
          collection: collections.find((c) => c.id === id),
          models,
          backdrops,
          patterns,
        },
      });
    },
  );
  const connections = new Set();
  const catalogPeers = new Set();
  const openSockets = new Set();
  let floorRequests = 0;
  let floorPrice = "8";
  const catalogCommands = [];
  let socketCount = 0,
    orderSubscriptions = 0;
  const sendCatalog = (peer) =>
    peer.socket.send(
      JSON.stringify({
        type: "catalog.versions",
        id: peer.id,
        versions: {
          directory: directoryRevision,
          collections: Object.fromEntries(
            peer.ids.map((id) => [id, collectionRevisions[id] ?? null]),
          ),
        },
      }),
    );
  const notifyCatalog = () => {
    for (const peer of catalogPeers) sendCatalog(peer);
  };
  await page.route("**/valuations/strategies", (route) =>
    route.fulfill({
      json: [
        {
          purpose: "estimate",
          id: "gift_floor",
          revision: "r1",
          display: fixtureDisplay("gift_floor"),
          input: {
            targets: ["gift", "gift_attributes"],
            period: "unsupported",
          },
          name: "Gift floor",
          description: "Gift floor estimate",
          documentation_url: "/valuations/strategies/gift_floor/documentation",
          documentation_revision: "r1",
        },
        {
          purpose: "reference",
          id: "ask_floor",
          revision: "r1",
          display: fixtureDisplay("ask_floor"),
          input: { targets: ["selection"], period: "unsupported" },
          name: "Matching asks",
          description: "Matching selection",
          documentation_url: "/guide/ask_floor",
          documentation_revision: "r1",
        },
      ],
    }),
  );
  const valuationReport = (body) => {
    const number = body.gift.number;
    const collection = body.gift.collection_id;
    return {
      strategy: "gift_floor",
      stale: false,
      evaluated_at: "2026-11-18T12:00:00Z",
      slug: null,
      gift: {
        id: `gift-${3000 - number}`,
        collection_id: collection,
        number,
        traits: { model: "barbie", backdrop: "raspberry", pattern: "stars" },
      },
      currency_id: body.currency_id,
      collection_floor: { ...{ value: "8" }, count: 5 },
      model_floor: { ...{ value: "10.000000001" }, count: 3 },
      backdrop_floor: { ...{ value: "9" }, count: 3 },
      model_backdrop_floor: { ...{ value: "12.5" }, count: 1 },
      model_multiplier: { value: "1.25" },
      backdrop_multiplier: { value: "1.13" },
      final_mark: { value: "11.25" },
    };
  };
  await page.route("**/valuations/query", () =>
    assert.fail("GUI valuation must use WebSocket"),
  );
  const snapshots = new Map();
  let snapshotSequence = 0;
  await page.route("**/orders/query", (route) => {
    const body = route.request().postDataJSON();
    if (route.request().method() === "DELETE") {
      snapshots.delete(body.snapshot);
      return route.fulfill({ status: 204 });
    }
    const id = body.view?.id ?? `snapshot.${++snapshotSequence}`;
    if (!snapshots.has(id))
      snapshots.set(id, result({ ...body, limit: 10000 }).data);
    return route.fulfill({
      json: {
        ...orderResult(body, snapshots.get(id)),
        snapshot: id,
      },
    });
  });
  await page.routeWebSocket("**/ws", (socket) => {
    socket.send(JSON.stringify({ type: "market.status", status: "available" }));
    socketCount++;
    openSockets.add(socket);
    let connection, catalogPeer;
    socket.onMessage((message) => {
      const frame = JSON.parse(String(message));
      if (frame.type === "cancel" || frame.type === "unsubscribe") {
        if (frame.id === connection?.id) connections.delete(connection);
        if (frame.id === catalogPeer?.id) catalogPeers.delete(catalogPeer);
      }
      if (frame.type === "request") {
        if (
          frame.method === "valuation.query" &&
          frame.params.target?.kind === "catalog"
        ) {
          floorRequests++;
          const data = floorReport(
            frame.params,
            models.map((row) => row.id),
            backdrops.map((row) => row.id),
            patterns.map((row) => row.id),
          );
          for (const row of data.results[0].data.collections) {
            row.floor.value = floorPrice;
            if (floorPrice === "7") row.floor.count = 8;
          }
          socket.send(
            JSON.stringify({
              type: "result",
              id: frame.id,
              method: frame.method,
              data,
            }),
          );
          return;
        }
        socket.send(
          JSON.stringify({
            type: "result",
            id: frame.id,
            method: frame.method,
            ...(frame.method === "orders.query"
              ? orderResult(
                  frame.params,
                  result({ ...frame.params, limit: 10000 }).data,
                )
              : {
                  data:
                    frame.params.target?.kind === "selection"
                      ? criteriaReport(frame.params)
                      : giftReport(
                          frame.params,
                          valuationReport({
                            gift:
                              frame.params.target.gift ?? frame.params.target,
                            currency_id: frame.params.currency_id,
                          }),
                        ),
                }),
          }),
        );
      }
      if (frame.type === "catalog.watch") {
        catalogCommands.push(frame.collection_ids);
        catalogPeers.delete(catalogPeer);
        catalogPeer = { socket, id: frame.id, ids: frame.collection_ids };
        catalogPeers.add(catalogPeer);
        sendCatalog(catalogPeer);
      }
      if (frame.type === "orders.watch") {
        orderSubscriptions++;
        connections.delete(connection);
        const rows = result({ ...frame.params, limit: 10000 }).data;
        connection = {
          socket,
          id: frame.id,
          query: frame.params,
          watch: new RangeWatch(frame.params, rows),
        };
        connections.add(connection);
        socket.send(JSON.stringify(connection.watch.frame(frame.id, rows)));
      }
    });
    socket.onClose(() => {
      connections.delete(connection);
      catalogPeers.delete(catalogPeer);
      openSockets.delete(socket);
    });
  });
  const base = `http://127.0.0.1:${server.httpServer.address().port}`;
  await page.setViewportSize({ width: 1200, height: 900 });
  await page.goto(`${base}/?collection=bunny&currency=GRAM`);
  const title = page.locator(".market-heading h1");
  await page
    .getByRole("heading", { name: "Loading collection", exact: true })
    .waitFor();
  assert.equal(await title.getAttribute("aria-busy"), "true");
  assert.equal(await title.locator(".collection-title-skeleton").count(), 1);
  assert.equal(
    await page.getByText("Select a collection", { exact: true }).count(),
    0,
    "Selected collections never flash an empty-selection title",
  );
  releaseInitialCatalog();
  await page
    .getByRole("heading", { name: "Jelly Bunny", exact: true })
    .waitFor();
  assert.equal(await title.getAttribute("aria-busy"), "false");
  assert.equal(await title.locator(".collection-title-skeleton").count(), 0);
  assert.equal(await page.locator(".market-eyebrow").count(), 0);
  assert.equal(await page.locator(".feed-status").count(), 1);
  const headerLayout = await page
    .locator(".market-heading")
    .evaluate((header) => {
      const title = header.querySelector("h1").getBoundingClientRect();
      const status = header
        .querySelector(".feed-status")
        .getBoundingClientRect();
      return {
        oneRow:
          Math.abs(
            (title.top + title.bottom) / 2 - (status.top + status.bottom) / 2,
          ) < 1,
        separated: status.left >= title.right + 15,
        compact: header.getBoundingClientRect().height <= 40,
      };
    });
  assert.deepEqual(headerLayout, {
    oneRow: true,
    separated: true,
    compact: true,
  });
  assert.equal(
    await page.locator('.feed-status > [aria-hidden="true"]').innerText(),
    "·",
  );
  await page.setViewportSize({ width: 390, height: 844 });

  await page.waitForSelector('.collection-tile[data-collection-id="bunny"]');
  assert.deepEqual(
    catalogCommands,
    [["bunny"]],
    "Known initial scope starts one catalog watch",
  );
  const button = (name) => page.getByRole("button", { name, exact: true });
  const cards = (n) =>
    page
      .waitForFunction(
        (n) =>
          Number(document.querySelector(".gift-grid")?.dataset.orderCount) ===
            n &&
          document.querySelectorAll(".gift-card").length <= Math.min(n, 75),
        n,
      )
      .catch(async (error) => {
        console.error("UI state:", await page.locator("body").innerText());
        console.error("Browser errors:", errors);
        throw error;
      });
  const price = (value) =>
    page.waitForFunction(
      (value) =>
        document.querySelector(".gift-grid .quote-amount")?.textContent ===
        value,
      value,
    );
  const currency = async (value) => {
    await page.getByRole("combobox", { name: "Currency", exact: true }).click();
    await page
      .getByRole("option", {
        name: value === "XTR" ? "Stars (XTR)" : value,
        exact: true,
      })
      .click();
  };
  const scrollMore = async (count) => {
    if (
      (await page
        .locator(".gift-grid")
        .evaluate((el) => Number(el.dataset.orderCount))) >= count
    )
      return;
    // Trait panels and virtualization can reflow after the first scroll.
    for (let attempt = 0; attempt < 4; attempt++) {
      await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
      await page.mouse.wheel(0, 300);
      await page.waitForTimeout(150);
      if (
        (await page
          .locator(".gift-grid")
          .evaluate((el) => Number(el.dataset.orderCount))) >= count
      )
        return;
    }
    await cards(count);
  };
  await cards(80);
  assert.equal(
    await button("Back to top").count(),
    0,
    "Shortcut stays hidden while controls are visible",
  );
  await scrollMore(125);
  await button("Back to top").waitFor();
  const shortcutBox = await button("Back to top").boundingBox();
  assert.ok(shortcutBox.x + shortcutBox.width <= 390 - 16);
  assert.ok(shortcutBox.y + shortcutBox.height <= 844 - 16);
  assert.equal(shortcutBox.width, 44);
  assert.equal(shortcutBox.height, 44);
  assert.equal((await button("Back to top").textContent()).trim(), "");
  await page.screenshot({
    path: "/tmp/gifts-market-ui/mobile-back-to-top.png",
  });
  await page.setViewportSize({ width: 1200, height: 900 });
  await button("Back to top").waitFor({ state: "hidden" });
  await page.setViewportSize({ width: 390, height: 844 });
  await button("Back to top").waitFor();
  const loadedBeforeShortcut = await page
    .locator(".gift-grid")
    .getAttribute("data-order-count");
  await page.evaluate(() => {
    const popup = document.createElement("dialog");
    popup.id = "shortcut-popup-test";
    document.body.append(popup);
    popup.showModal();
  });
  assert.equal(
    await button("Back to top").isVisible(),
    false,
    "Open popups hide the shortcut",
  );
  await page.evaluate(() => {
    const popup = document.getElementById("shortcut-popup-test");
    popup.close();
    popup.remove();
  });
  await button("Back to top").click();
  await page.waitForFunction(
    () =>
      document.activeElement?.id === "market-filters" &&
      window.scrollY === 0 &&
      document.querySelector(".market-sidebar").getBoundingClientRect().top >=
        0,
  );
  await button("Back to top").waitFor({ state: "detached" });
  assert.equal(await button("Back to top").count(), 0);
  assert.equal(
    await page.locator(".gift-grid").getAttribute("data-order-count"),
    loadedBeforeShortcut,
    "Returning to the top preserves loaded listings",
  );
  await price("0.50..");
  assert.equal(
    await page
      .locator(".gift-grid .quote-amount")
      .first()
      .getAttribute("title"),
    "0.500000001 GRAM",
    "Cards retain the full quote on hover",
  );
  await page
    .getByRole("alert")
    .filter({ hasText: "Choices unavailable" })
    .waitFor();
  await button("Reload").click();
  await page.waitForFunction(() => !document.querySelector('[role="alert"]'));
  assert.equal(optionRequests, 2, "Failed choice loads expose a working retry");
  const sortTrigger = page.getByRole("combobox", {
    name: "Sort by",
    exact: true,
  });
  assert.equal(new URL(page.url()).searchParams.get("sort"), "price_asc");
  assert.equal(await sortTrigger.innerText(), "Price");
  assert.equal(await button("Reverse sort direction").innerText(), "↑");
  await sortTrigger.focus();
  await page.keyboard.press("ArrowDown");
  await page
    .getByRole("listbox", { name: "Sort by options", exact: true })
    .waitFor();
  await page.keyboard.press("Home");
  assert.equal(
    await page
      .getByRole("option", { name: "Time", exact: true })
      .evaluate((el) => el === document.activeElement),
    true,
  );
  await page.keyboard.press("End");
  assert.equal(
    await page
      .getByRole("option", { name: "Price", exact: true })
      .evaluate((el) => el === document.activeElement),
    true,
  );
  await page.keyboard.press("Escape");
  assert.equal(await sortTrigger.getAttribute("aria-expanded"), "false");
  assert.equal(
    await sortTrigger.evaluate((el) => el === document.activeElement),
    true,
  );
  await sortTrigger.click();
  await button("Asks").click();
  assert.equal(
    await sortTrigger.getAttribute("aria-expanded"),
    "false",
    "Sort menu dismisses on outside click",
  );
  const sort = async (label) => {
    await page.getByRole("combobox", { name: "Sort by", exact: true }).click();
    await page.getByRole("option", { name: label, exact: true }).click();
  };
  assert.equal(
    await page.locator(".currency-picker .picker-trigger").innerText(),
    "",
  );
  assert.equal(
    await page
      .locator(".gift-card")
      .first()
      .getByRole("img", { name: "GRAM", exact: true })
      .count(),
    1,
  );
  assert.equal(
    await page
      .locator(".collection-rail")
      .evaluate(
        (el) =>
          Math.abs(
            document.querySelector(".pickers").getBoundingClientRect().top -
              el.getBoundingClientRect().bottom -
              8,
          ) < 1,
      ),
    true,
    "Collection and attribute controls have an eight-pixel gap",
  );
  const mrktIcon = page.locator(".gift-card .venue-logo").first();
  await mrktIcon.waitFor();
  await page.waitForFunction(
    () => document.querySelector(".gift-card .venue-logo")?.naturalWidth > 0,
  );
  assert.equal(
    await mrktIcon.evaluate((el) => {
      const bounds = el.getBoundingClientRect(),
        parent = el.parentElement.getBoundingClientRect();
      return (
        getComputedStyle(el).opacity === "1" &&
        getComputedStyle(el).position === "static" &&
        bounds.width <= parent.width &&
        bounds.height <= parent.height
      );
    }),
    true,
    "MRKT logos stay visible and contained inside gift cards",
  );
  assert.equal(
    await button("Asks").evaluate((el) => getComputedStyle(el).fontSize),
    "13px",
  );
  assert.equal(
    await page.locator(".trait-preview").count(),
    0,
    "Unselected traits must not invent a default preview",
  );
  assert.equal(await button("Filters").count(), 0);
  assert.equal(await page.getByLabel("Rows per page").count(), 0);
  assert.equal(await button("Next →").count(), 0);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await button("Plush Pepe").click();
  await page.waitForFunction(() => {
    const tiles = [...document.querySelectorAll(".collection-tile")];
    const animations = tiles
      .flatMap((tile) => tile.getAnimations())
      .filter((animation) => animation.transitionProperty === "width");
    if (animations.length !== 2) return false;
    for (const animation of animations) {
      animation.pause();
      animation.currentTime = 110;
    }
    return true;
  });
  assert.equal(
    await page
      .locator('.collection-tile:not([data-collection-id=""])')
      .evaluateAll((tiles) =>
        tiles.every((tile) => {
          const width = tile.getBoundingClientRect().width;
          return width > 60 && width < 148;
        }),
      ),
    true,
    "Both collection tiles interpolate their width instead of jumping to their minimum",
  );
  await page
    .locator(".collection-tile")
    .evaluateAll((tiles) =>
      tiles.forEach((tile) =>
        tile
          .getAnimations({ subtree: true })
          .forEach((animation) => animation.finish()),
      ),
    );
  await page.emulateMedia({ reducedMotion: "reduce" });
  await button("Jelly Bunny").click();
  await cards(80);
  await scrollMore(125);
  await page.evaluate(() => window.scrollTo(0, 0));
  await price("0.50..");
  await scrollMore(80);
  await scrollMore(125);
  assert.equal(
    new Set(
      await page
        .locator(".card-open")
        .evaluateAll((elements) => elements.map((el) => el.dataset.orderId)),
    ).size,
    await page.locator(".card-open").count(),
  );
  assert.ok(
    (await page.locator(".gift-card").count()) < 60,
    "Only viewport rows should be mounted",
  );
  assert.equal(
    await page.getByText("All gifts loaded", { exact: true }).count(),
    0,
  );
  assert.equal(
    await page.locator(".scroll-loader").count(),
    0,
    "Completed books remove the scroll sentinel and its end label",
  );
  assert.equal(await button("Ⅱ Pause").count(), 0);
  assert.equal(await button("Refresh market").count(), 0);
  await button("Search collections").click();
  await page.getByLabel("Filter collections").fill("pepe");
  assert.equal(await page.locator(".collection-tile").count(), 1);
  await page.getByLabel("Filter collections").press("Enter");
  await page.waitForURL(/f=c%3Dpepe/);
  await cards(80);
  await scrollMore(125);
  await page.evaluate(() => window.scrollTo(0, 0));
  assert.equal(
    await page.locator(".collection-rail").evaluate((rail) => {
      const search = rail.querySelector(".search-tile").getBoundingClientRect();
      const viewport = rail
        .querySelector(".collection-list")
        .getBoundingClientRect();
      const selectedTile = rail.querySelector(
        '.collection-tile[aria-pressed="true"]',
      );
      const selected = selectedTile.getBoundingClientRect();
      const previous =
        selectedTile.previousElementSibling.getBoundingClientRect();
      return (
        Math.abs(viewport.left - search.right - 7) < 1 &&
        Math.abs(previous.left - viewport.left) < 1 &&
        Math.abs(selected.left - previous.right - 7) < 1
      );
    }),
    true,
    "The selected collection occupies the second visible slot after search",
  );
  await button("Search collections").click();
  await page.keyboard.press("Escape");
  assert.equal(
    await button("Search collections").evaluate(
      (el) => el === document.activeElement,
    ),
    true,
  );
  await button("Search collections").click();
  await page.getByLabel("Filter collections").fill("bunny");
  await page.getByLabel("Filter collections").press("Enter");
  await cards(80);
  await scrollMore(125);
  await page.evaluate(() => window.scrollTo(0, 0));
  assert.equal(
    optionRequests,
    3,
    "Returning to a loaded collection reuses its complete metadata",
  );
  // Every selected trait must appear in the composition, not only the first three.
  await button("Model").click();
  await page.locator(".trait-option .floor-badge").first().waitFor();
  const beforeTraitFloors = floorRequests;
  for (const model of models) await button(model.name).click();
  await page.waitForTimeout(100);
  assert.equal(
    floorRequests,
    beforeTraitFloors,
    "Trait selection reuses the existing batched floor watch",
  );
  await page.keyboard.press("Escape");
  assert.equal(await page.locator(".preview-model").count(), 5);
  assert.equal(
    await page.locator(".trait-preview").evaluate((el) => {
      const box = el.getBoundingClientRect();
      const reset = el.querySelector(".reset-traits").getBoundingClientRect();
      return [...el.querySelectorAll(".preview-model > span")].every(
        (label) => {
          const bounds = label.getBoundingClientRect();
          return (
            bounds.top >= box.top &&
            bounds.bottom <= box.bottom &&
            (bounds.right <= reset.left ||
              bounds.left >= reset.right ||
              bounds.bottom <= reset.top)
          );
        },
      );
    }),
    true,
    "Selected model labels stay inside the banner and clear of Reset traits",
  );
  await button("Backdrop").click();
  for (const backdrop of backdrops) await button(backdrop.name).click();
  await page.keyboard.press("Escape");
  assert.equal(await page.locator(".preview-band").count(), 5);
  await button("Symbol").click();
  for (const pattern of patterns) await button(pattern.name).click();
  await page.keyboard.press("Escape");
  await cards(80);
  await scrollMore(125);
  await page.evaluate(() => window.scrollTo(0, 0));
  await button("Reset traits").click();
  await cards(80);
  await scrollMore(125);
  await page.evaluate(() => window.scrollTo(0, 0));
  await button("Model").click();
  await button("Barbie").click();
  await cards(25);
  await page.keyboard.press("Escape");
  await page.reload();
  await cards(25);
  assert.ok(
    new URL(page.url()).searchParams.get("f")?.includes("m=@name:Barbie"),
  );
  await button("Reset traits").click();
  await cards(80);
  await scrollMore(125);
  await page.evaluate(() => window.scrollTo(0, 0));
  await button("Telegram").click();
  await cards(42);
  await button("All venues").click();
  await cards(80);
  await scrollMore(125);
  await page.evaluate(() => window.scrollTo(0, 0));
  await button("Reverse sort direction").click();
  await price("10,000,000,000,000,000.00..");
  for (const width of [320, 390, 480, 1440]) {
    await page.setViewportSize({ width, height: 844 });
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
      false,
      `No overflow at ${width}`,
    );
    const columns = await page
      .locator(".gift-grid")
      .evaluate(
        (el) => getComputedStyle(el).gridTemplateColumns.split(" ").length,
      );
    assert.ok(
      width >= 960 ? columns >= 4 : columns === 3,
      `Responsive columns at ${width}: ${columns}`,
    );
  }
  // The desktop sidebar owns all market controls, while the document still
  // scrolls the virtual listings. Resizing must retain selection and card DOM.
  const sidebar = page.getByRole("complementary", { name: "Market controls" });
  const desktopGeometry = await page.evaluate(() => {
    const sidebar = document
      .querySelector(".market-sidebar")
      .getBoundingClientRect();
    const grid = document.querySelector(".gift-grid").getBoundingClientRect();
    window.responsiveCard = document.querySelector(".card-open");
    return {
      sidebarRight: sidebar.right,
      gridLeft: grid.left,
      gridWidth: grid.width,
      count: document.querySelector(".gift-grid").dataset.orderCount,
    };
  });
  assert.ok(desktopGeometry.gridLeft > desktopGeometry.sidebarRight);
  assert.ok(desktopGeometry.gridWidth > 700);
  assert.equal(
    await sidebar
      .getByRole("button", { name: "Refresh market", exact: true })
      .count(),
    0,
  );
  assert.equal(
    await sidebar
      .getByRole("combobox", { name: "Sort by", exact: true })
      .count(),
    1,
  );
  assert.equal(
    await page.getByRole("combobox", { name: "Currency", exact: true }).count(),
    1,
  );
  assert.equal(
    await sidebar.locator('.sidebar-heading [role="combobox"]').count(),
    1,
  );
  assert.equal(
    await sidebar
      .locator(".sidebar-heading")
      .getByText("Market", { exact: true })
      .count(),
    0,
  );
  assert.equal(
    await sidebar.getByRole("link", { name: "Market", exact: true }).count(),
    1,
  );
  assert.equal(await sidebar.locator(".feed-actions").count(), 0);
  assert.equal(
    await sidebar
      .locator(".collection-list")
      .evaluate((element) => getComputedStyle(element).flexDirection),
    "column",
  );
  await page.evaluate(() => window.scrollTo(0, 350));
  assert.ok(
    await sidebar.evaluate((node) => node.getBoundingClientRect().top >= 19),
    "Controls remain sticky while listings scroll",
  );
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.setViewportSize({ width: 1024, height: 560 });
  await button("Model").click();
  const beforeSidebarScroll = await page.evaluate(() => window.scrollY);
  await sidebar.evaluate((node) => {
    node.scrollTop = node.scrollHeight;
  });
  assert.equal(
    await page.evaluate(() => window.scrollY),
    beforeSidebarScroll,
    "Sidebar scrolling does not scroll or paginate the listings",
  );
  await page.getByRole("combobox", { name: "Sort by", exact: true }).click();
  await page.getByRole("option", { name: "Price", exact: true }).click();
  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForFunction(
    (count) =>
      document.querySelector(".gift-grid")?.dataset.orderCount === count,
    desktopGeometry.count,
  );
  assert.equal(
    await page.evaluate(
      () => document.querySelector(".card-open") === window.responsiveCard,
    ),
    true,
    "Responsive reflow retains card DOM and media",
  );
  await sort("Gift #");
  await page.waitForFunction(
    () =>
      document.querySelector(".gift-card .serial")?.textContent ===
      "#2\u00a0000",
  );
  await sort("Time");
  await page.waitForFunction(
    () =>
      document.querySelector(".gift-card .serial")?.textContent ===
      "#1\u00a0876",
  );
  await sort("Price");
  await button("Reverse sort direction").click();
  await price("0.50..");
  await currency("XTR");
  await price("850");
  assert.equal(
    await page.locator(".gift-card").first().locator(".star").count(),
    1,
  );
  await currency("GRAM");
  await price("0.50..");
  await page.goto(
    `${base}/?collection=bunny&currency=GRAM&sort=price_asc&max=12`,
  );
  await cards(3);
  await currency("XTR");
  await cards(80);
  await scrollMore(125);
  await page.evaluate(() => window.scrollTo(0, 0));
  assert.equal(
    new URL(page.url()).searchParams.has("max"),
    false,
    "Changing units clears a bound denominated in the old currency",
  );
  await currency("GRAM");
  await price("0.50..");
  await button("Bids").click();
  await cards(1);
  const bidViewport = page.viewportSize();
  for (const width of [320, 390, 1200]) {
    await page.setViewportSize({ width, height: 900 });
    assert.deepEqual(
      await page.locator(".market-controls").evaluate((control) => {
        const views = control.querySelector(".market-views");
        const bid = views.querySelector("button.active");
        const panel = control.querySelector(".bid-matching");
        const bounds = panel.getBoundingClientRect();
        return {
          attached:
            Math.abs(views.getBoundingClientRect().bottom - bounds.top) < 1,
          sameBackground:
            getComputedStyle(bid).backgroundColor ===
            getComputedStyle(panel).backgroundColor,
          linked: bid.getAttribute("aria-controls") === panel.id,
          contained: [...panel.querySelectorAll("button")].every((button) => {
            const box = button.getBoundingClientRect();
            return box.left >= bounds.left && box.right <= bounds.right;
          }),
          overflow: document.documentElement.scrollWidth > innerWidth,
        };
      }),
      {
        attached: true,
        sameBackground: true,
        linked: true,
        contained: true,
        overflow: false,
      },
    );
    await page.screenshot({
      path: `/tmp/gifts-market-ui/bids-controls-${width}.png`,
    });
    assert.equal(
      await page.locator(".sort-control").evaluate((control) => {
        const picker = control
          .querySelector(".picker-trigger")
          .getBoundingClientRect();
        const button = control.querySelector(":scope > button");
        const direction = button.getBoundingClientRect();
        const separator = getComputedStyle(button, "::before");
        return (
          Math.abs(picker.top - direction.top) < 1 &&
          Math.abs(picker.height - direction.height) < 1 &&
          Math.abs(picker.height - control.clientHeight) < 1 &&
          separator.width === "1px" &&
          parseFloat(separator.top) > 0 &&
          parseFloat(separator.bottom) > 0
        );
      }),
      true,
      "Sort separator stays inset and both buttons share the same height",
    );
  }
  await page.setViewportSize(bidViewport);
  assert.equal(
    await page
      .getByRole("group", { name: "Bid matching", exact: true })
      .getByRole("button", { name: "Exact", exact: true })
      .getAttribute("aria-pressed"),
    "true",
  );
  await page
    .getByRole("group", { name: "Bid matching", exact: true })
    .getByRole("button", { name: "Compatible", exact: true })
    .click();
  await cards(4);
  await price("13");
  assert.equal(new URL(page.url()).searchParams.get("sort"), "price_desc");
  assert.equal(await sortTrigger.innerText(), "Price");
  assert.equal(await button("Reverse sort direction").innerText(), "↓");
  assert.equal(
    await page
      .locator(".app-footer")
      .evaluate(
        (footer) =>
          Math.abs(
            footer.getBoundingClientRect().bottom -
              (innerHeight -
                parseFloat(
                  getComputedStyle(footer.closest(".orderbook")).paddingBottom,
                )),
          ) < 2,
      ),
    true,
    "Short books keep the footer at the viewport bottom: " +
      JSON.stringify(
        await page.locator(".app-footer").evaluate((footer) => ({
          footer: footer.getBoundingClientRect().toJSON(),
          height: innerHeight,
          scrollY,
          padding: getComputedStyle(footer.closest(".orderbook")).paddingBottom,
          main: footer.closest(".orderbook").getBoundingClientRect().toJSON(),
        })),
      ),
  );
  assert.equal(await button("Model").isDisabled(), false);
  assert.equal(
    await page
      .locator(".gift-card")
      .filter({ has: page.locator('[data-order-id="order-0"]') })
      .getByText("Unsupported conditions", { exact: true })
      .count(),
    1,
  );
  await page.locator('[data-order-id="order-0"]').click();
  await page
    .locator(".detail")
    .getByText("This listing has conditions that cannot be displayed yet.", {
      exact: true,
    })
    .waitFor();
  assert.equal(await page.locator(".detail-market").count(), 0);
  assert.equal(
    await page.locator(".detail .detail-venue").isVisible(),
    true,
    "Venue remains visible without a market URL",
  );
  assert.equal(
    await page
      .locator(".detail :is(dt, th)")
      .getByText("Model", { exact: true })
      .count(),
    0,
  );
  await page.keyboard.press("Escape");
  await page.locator('[data-order-id="order-1"]').click();
  await page
    .locator(".detail")
    .getByLabel("Quantity: 5", { exact: true })
    .waitFor();
  assert.equal(
    await page.locator('.detail [data-condition="quantity"]').count(),
    0,
  );
  await page.keyboard.press("Escape");
  const matching = page.getByRole("group", {
    name: "Bid matching",
    exact: true,
  });
  assert.equal(
    await page.locator(".bid-matching > span,.bid-matching > p").count(),
    0,
  );

  assert.equal(
    await matching
      .getByRole("button", { name: "Compatible", exact: true })
      .getAttribute("aria-pressed"),
    "true",
  );
  await button("Model").click();
  const modelChoices = page.getByRole("group", {
    name: "Model options",
    exact: true,
  });
  await modelChoices
    .getByRole("button", { name: "Barbie", exact: true })
    .click();
  await modelChoices
    .getByRole("button", { name: "Juicy Mint", exact: true })
    .click();
  await cards(3);
  await matching.getByRole("button", { name: "Exact", exact: true }).click();
  await cards(1);
  assert.equal(new URL(page.url()).searchParams.get("bid_match"), "all");
  assert.equal([...connections].at(-1).query.bid_match, "all");
  const beforeBidReconnect = socketCount;
  [...openSockets][0].close();
  for (let i = 0; i < 100 && socketCount === beforeBidReconnect; i++)
    await new Promise((resolve) => setTimeout(resolve, 25));
  await cards(1);
  assert.ok(socketCount > beforeBidReconnect);
  assert.equal([...connections].at(-1).query.bid_match, "all");
  await page.reload();
  await cards(1);
  assert.equal(
    await matching
      .getByRole("button", { name: "Exact", exact: true })
      .getAttribute("aria-pressed"),
    "true",
  );
  await button("Reset traits").click();
  await cards(1);
  await matching
    .getByRole("button", { name: "Compatible", exact: true })
    .click();
  await cards(4);
  assert.equal(new URL(page.url()).searchParams.get("bid_match"), "any");
  await page.reload();
  await cards(4);
  assert.equal(
    await matching
      .getByRole("button", { name: "Compatible", exact: true })
      .getAttribute("aria-pressed"),
    "true",
  );
  await button("History").click();
  assert.equal(await page.locator(".bid-matching").count(), 0);
  await cards(75);
  assert.equal(new URL(page.url()).searchParams.get("sort"), "time_desc");
  assert.equal(await sortTrigger.innerText(), "Time");
  assert.equal(await button("Reverse sort direction").innerText(), "↓");
  await page.waitForFunction(
    () =>
      document.querySelector(".gift-card .serial")?.textContent ===
      "#1\u00a0926",
  );
  await page.locator(".feed-status .status.live").waitFor();
  await scrollMore(75);
  assert.equal(snapshotSequence, 0, "GUI history uses no frozen REST lease");
  await page.evaluate(() => window.scrollTo(0, 700));
  await page.waitForTimeout(150);
  const anchor = await page.locator(".card-open").evaluateAll((nodes) => {
    const node = nodes.find((node) => {
      const r = node.getBoundingClientRect();
      return r.bottom > 0 && r.top < innerHeight;
    });
    return { id: node.dataset.orderId, top: node.getBoundingClientRect().top };
  });
  const historyPeer = [...connections].find(
    (candidate) => candidate.query.source === "history",
  );
  assert.ok(historyPeer, "History has an active orders.watch");
  const template = result({ ...historyPeer.query, limit: 10000 }).data[0];
  historyExtras = [
    {
      ...template,
      id: "execution-new",
      target: { ...template.target, id: "gift-1100", number: 1900 },
      executed_at: new Date(
        Date.parse(template.executed_at) + 1000,
      ).toISOString(),
    },
    {
      ...template,
      id: "execution-backfill",
      target: { ...template.target, id: "gift-1099", number: 1901 },
      executed_at: new Date(
        Date.parse(template.executed_at) - 5500,
      ).toISOString(),
    },
  ];
  historyPeer.socket.send(
    JSON.stringify(
      historyPeer.watch.frame(
        historyPeer.id,
        result({ ...historyPeer.query, limit: 10000 }).data,
      ),
    ),
  );
  await cards(77);
  const top = await page
    .locator(`[data-order-id="${anchor.id}"]`)
    .evaluate((node) => node.getBoundingClientRect().top);
  assert.ok(
    Math.abs(top - anchor.top) < 2,
    "Live execution inserts preserve the visible scroll anchor",
  );
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.waitForFunction(
    () =>
      document.querySelector(".gift-card .serial")?.textContent ===
      "#1\u00a0900",
  );
  assert.ok((await page.locator(".gift-card.is-new").count()) > 0);
  assert.equal(
    snapshotSequence,
    0,
    "Live history does not poll or open snapshot reads",
  );
  await page.locator(".card-open").first().click();
  await page
    .locator(".detail :is(dt, th)")
    .getByText("Model", { exact: true })
    .waitFor();
  assert.equal(
    await page
      .locator(".detail :is(dt, th)")
      .getByText("Last observed", { exact: true })
      .count(),
    0,
  );
  await page.keyboard.press("Escape");
  historyExtras = [];
  await button("Asks").click();
  await cards(80);
  await scrollMore(125);
  await page.evaluate(() => window.scrollTo(0, 0));
  await price("0.50..");
  assert.equal(new URL(page.url()).searchParams.get("sort"), "price_asc");
  const opener = page.locator(".card-open").first();
  await opener.click();
  await page.getByRole("dialog", { name: "Order details" }).waitFor();
  await page
    .locator(".detail :is(dt, th)")
    .getByText("Model", { exact: true })
    .waitFor();
  const labels = await page.locator(".detail :is(dt, th)").allTextContents();
  for (const hidden of [
    "Order ID",
    "Gift ID",
    "Parent order",
    "Mode",
    "Quantity",
    "Gift #",
    "Last observed",
    "Executed",
    "Expires",
    "Venue",
  ])
    assert.ok(!labels.includes(hidden), hidden);
  assert.equal(await page.locator(".source-details").count(), 0);
  await page.locator('[data-metric="final_mark"] .valuation-value').waitFor();
  await page.getByLabel("Collection floor: 8 GRAM", { exact: true }).waitFor();
  const beforeFloorUpdate = floorRequests;
  floorPrice = "7";
  for (const socket of openSockets) {
    socket.send(
      JSON.stringify({ type: "market.status", status: "unavailable" }),
    );
    socket.send(JSON.stringify({ type: "market.status", status: "available" }));
  }
  await page.getByLabel("Collection floor: 7 GRAM", { exact: true }).waitFor();
  assert.equal(
    await page
      .locator('[data-attribute="collection"] .floor-evidence')
      .first()
      .textContent(),
    "8 gifts",
  );
  const countTooltip = await page
    .locator('[data-attribute="collection"] .floor-evidence')
    .first()
    .getAttribute("title");
  assert.match(countTooltip, /^8 gifts\n/);
  assert.match(countTooltip, /Unique identifiable gifts/);
  assert.match(countTooltip, /same gift counts once across markets/);
  assert.doesNotMatch(countTooltip, /listings/);

  assert.ok(
    floorRequests > beforeFloorUpdate && floorRequests <= beforeFloorUpdate + 2,
    "Reconnect refreshes collection summaries and selected traits with bounded finite queries",
  );
  await page.screenshot({
    path: "/tmp/gifts-market-ui/shared-values-details.png",
  });
  assert.equal(await page.locator(".valuation-breakdown").count(), 0);
  await page.locator('[data-attribute="model"] .floor-badge').waitFor();
  assert.equal(
    await page
      .getByRole("button", { name: "How is this calculated?", exact: true })
      .count(),
    1,
  );
  assert.equal(
    await button("Close details").evaluate(
      (el) => el === document.activeElement,
    ),
    true,
  );
  assert.equal(await page.locator(".valuation-metrics > li").count(), 1);
  assert.equal(await page.locator(".gift-attributes [data-metric]").count(), 0);
  assert.equal(
    await page
      .locator('[data-attribute="model"] .floor-evidence')
      .textContent(),
    "2 gifts",
  );
  assert.equal(
    await page
      .locator('[data-attribute="backdrop"] .floor-evidence')
      .textContent(),
    "1 gift",
  );
  assert.equal(
    await page.locator('[data-attribute="pattern"] .floor-summary').count(),
    1,
  );
  assert.equal(
    await page
      .locator('[data-metric="final_mark"] .quote-amount')
      .textContent(),
    "11.25",
  );
  assert.deepEqual(
    await page
      .locator(".detail-attributes tbody > tr")
      .evaluateAll((rows) => rows.map((row) => row.dataset.attribute)),
    ["collection", "pair", "model", "backdrop", "pattern"],
  );
  assert.equal(
    await page
      .locator('[data-attribute="pattern"] .quote-amount')
      .textContent(),
    "8.5",
  );
  assert.equal(
    await page
      .locator('[data-attribute="pattern"] .floor-evidence')
      .textContent(),
    "3 gifts",
  );
  assert.equal(
    await page
      .locator(".detail-pricing")
      .getByRole("heading", { name: "Asking price" })
      .count(),
    0,
  );
  const inspect = page.locator(
    '[data-attribute="model"] .floor-value:not(.bid-value)',
  );
  const beforeInspect = floorRequests;
  await inspect.click();
  const floorInfo = page.getByRole("dialog", {
    name: "Floor details",
    exact: true,
  });
  assert.equal(await floorInfo.count(), 0);
  assert.equal(await inspect.evaluate((el) => el.tagName), "SPAN");
  assert.equal(
    await page
      .locator(".gift-attributes button, .gift-attributes [aria-haspopup]")
      .count(),
    0,
  );
  assert.equal(
    floorRequests,
    beforeInspect,
    "Read-only floor values reuse the loaded projection",
  );
  assert.equal(
    await page.locator(".detail").count(),
    1,
    "Reading a floor keeps the gift open",
  );
  const detailViewport = page.viewportSize();
  for (const width of [320, 390, 1200]) {
    await page.setViewportSize({ width, height: 844 });
    const priceLayout = await page
      .locator(".listing-price-row")
      .evaluate((row) => {
        const label = row.querySelector("h3").getBoundingClientRect();
        const price = row
          .querySelector(".detail-price")
          .getBoundingClientRect();
        return (
          label.right <= price.left &&
          Math.abs(price.right - row.getBoundingClientRect().right) < 1
        );
      });
    assert.equal(priceLayout, true, "Ask price ends at the right edge");
    const centered = await page.locator(".attribute-pair").evaluate((pair) => {
      const backdrop = pair
        .querySelector(":scope > .swatch")
        .getBoundingClientRect();
      const model = pair
        .querySelector(".attribute-pair-model .gift-model-image")
        .getBoundingClientRect();
      return (
        Math.abs(backdrop.x + backdrop.width / 2 - model.x - model.width / 2) <
          0.5 &&
        Math.abs(
          backdrop.y + backdrop.height / 2 - model.y - model.height / 2,
        ) < 0.5
      );
    });
    assert.equal(centered, true, "The model is centered over its backdrop");
    const attributeLayout = await page
      .locator(".detail-attributes")
      .evaluate((list) =>
        [...list.querySelectorAll("tbody > tr")].map((row) => {
          const bounds = row.getBoundingClientRect();
          const label = row
            .querySelector("th .attribute-identity")
            .getBoundingClientRect();
          const floor = row.querySelector("td").getBoundingClientRect();
          return {
            compact: bounds.height <= 54,
            separate: label.right <= floor.left,
            contained: floor.right <= bounds.right && label.left >= bounds.left,
          };
        }),
      );
    assert.ok(
      attributeLayout.every(
        (row) => row.compact && row.separate && row.contained,
      ),
      `Attribute names and floors fit at ${width}px: ${JSON.stringify(attributeLayout)}`,
    );
  }
  await page.setViewportSize(detailViewport);
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => !document.querySelector(".detail"));
  assert.equal(
    await opener.evaluate((el) => el === document.activeElement),
    true,
  );
  await opener.click();
  await page.mouse.click(5, 5);
  await page.waitForFunction(() => !document.querySelector(".detail"));
  // Exercise live metadata inside Telegram's Mini App context, with no account credentials.
  await page.route("**/telegram", (route) =>
    route.fulfill({
      json: {
        auth_enabled: false,
        bot_id: null,
        bot_username: null,
        miniapp_url: null,
      },
    }),
  );
  await page.addInitScript(() => {
    window.Telegram = {
      WebApp: {
        initData: "fixture",
        colorScheme: "dark",
        themeParams: {},
        ready() {},
        expand() {},
        onEvent() {},
        offEvent() {},
        BackButton: { show() {}, hide() {}, onClick() {}, offClick() {} },
      },
    };
    window.catalogDocument = "same-document";
  });
  await page.reload();
  await cards(80);
  await scrollMore(125);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.waitForFunction(() =>
    document.documentElement.classList.contains("telegram"),
  );
  const choose = async (name) => {
    await button("Search collections").click();
    await page.getByLabel("Filter collections").fill(name);
    await page.getByLabel("Filter collections").press("Enter");
    await cards(80);
    await scrollMore(125);
    await page.evaluate(() => window.scrollTo(0, 0));
  };
  await choose("pepe");
  await choose("bunny");
  await new Promise((resolve) => setTimeout(resolve, 100));
  let beforeRequests = optionRequests;
  const beforeCatalog = catalogRequests;
  collectionRevisions.pepe = "a2aLbICR6oEhXOMFiOrwxw";
  notifyCatalog();
  await new Promise((resolve) => setTimeout(resolve, 150));
  assert.equal(optionRequests, beforeRequests, "Inactive changes remain lazy");
  assert.equal(openSockets.size, 1, "Catalog and orders share one socket");
  await button("History").click();
  await cards(75);
  models[0].name = "Live model";
  collectionRevisions.bunny = "kupF-YY6C-CXFMtJ4jmY6A";
  notifyCatalog();
  await button("Model").click();
  await button("Live model").waitFor();
  await page.keyboard.press("Escape");
  assert.equal(
    optionRequests,
    beforeRequests + 1,
    "Only the changed active collection downloads",
  );
  assert.equal(
    catalogRequests,
    beforeCatalog,
    "Trait changes do not download the directory",
  );
  assert.equal(
    await page.evaluate(() => window.catalogDocument),
    "same-document",
  );
  beforeRequests = optionRequests;
  const previousSockets = socketCount;
  [...catalogPeers][0].socket.close();
  for (let i = 0; i < 100 && socketCount === previousSockets; i++)
    await new Promise((resolve) => setTimeout(resolve, 25));
  await new Promise((resolve) => setTimeout(resolve, 150));
  assert.ok(socketCount > previousSockets);
  assert.equal(
    optionRequests,
    beforeRequests,
    "An unchanged reconnect reuses cached metadata",
  );
  models[0].name = "Updated while disconnected";
  collectionRevisions.bunny = "O7mGrcSR3nu-zTm_kIaNJw";
  [...catalogPeers][0].socket.close();
  await button("Model").click();
  await button("Updated while disconnected").waitFor();
  await page.keyboard.press("Escape");
  assert.equal(
    optionRequests,
    beforeRequests + 1,
    "Reconnect detects missed catalog changes",
  );
  assert.equal(openSockets.size, 1);
  assert.equal(
    await page.evaluate(() => window.catalogDocument),
    "same-document",
  );
  await button("Asks").click();
  await cards(80);
  await scrollMore(125);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.waitForTimeout(150);
  const healthyOrders = orderSubscriptions,
    healthySockets = socketCount;
  const beforeRecovery = optionRequests;
  let peer = [...catalogPeers][0];
  peer.socket.send(
    JSON.stringify({
      type: "error",
      id: peer.id,
      error: { code: "capacity_exceeded", message: "Catalog busy" },
    }),
  );
  catalogPeers.delete(peer);
  await page.getByRole("alert").filter({ hasText: "Catalog busy" }).waitFor();
  await page.waitForFunction(() => !document.querySelector('[role="alert"]'));
  assert.equal(
    socketCount,
    healthySockets,
    "A catalog retry preserves the socket",
  );
  assert.equal(
    orderSubscriptions,
    healthyOrders,
    "A catalog retry preserves healthy orders",
  );
  assert.equal(
    optionRequests,
    beforeRecovery,
    "Matching retry revisions preserve cached choices",
  );
  peer = [...catalogPeers][0];
  peer.socket.send(
    JSON.stringify({
      type: "error",
      id: peer.id,
      error: { code: "invalid_query", message: "Catalog needs retry" },
    }),
  );
  catalogPeers.delete(peer);
  await page
    .getByRole("alert")
    .filter({ hasText: "Catalog needs retry" })
    .waitFor();
  await button("Reload").click();
  await page.waitForFunction(() => !document.querySelector('[role="alert"]'));
  assert.equal(
    orderSubscriptions,
    healthyOrders,
    "Manual catalog recovery preserves healthy orders",
  );
  assert.equal(optionRequests, beforeRecovery);
  assert.equal(
    await page.evaluate(() => window.catalogDocument),
    "same-document",
  );
  // Direct links without a sort use the selected view's default. A linked
  // custom sort and clicking the already-active tab preserve the user's choice.
  for (const [search, count, expectedSort, label, direction] of [
    ["mode=bid", 1, "price_desc", "Price", "↓"],
    ["mode=executed", 75, "time_desc", "Time", "↓"],
    ["mode=bid&bid_match=any", 4, "price_desc", "Price", "↓"],
    ["mode=bid&sort=time_asc", 1, "time_asc", "Time", "↑"],
  ]) {
    await page.goto(`${base}/?collection=bunny&currency=GRAM&${search}`);
    await cards(count);
    assert.equal(new URL(page.url()).searchParams.get("sort"), expectedSort);
    assert.equal(await sortTrigger.innerText(), label);
    assert.equal(await button("Reverse sort direction").innerText(), direction);
  }
  await button("Bids").click();
  assert.equal(new URL(page.url()).searchParams.get("sort"), "time_asc");
  // Startup and old All gifts links choose one collection before opening a feed.
  for (const width of [390, 1200]) {
    await page.setViewportSize({ width, height: 900 });
    for (const search of [
      "",
      "?f=&mode=bid",
      "?f=b%3D%40name%3ARaspberry&mode=executed",
    ]) {
      await page.goto(`${base}/${search}`);
      await page
        .getByRole("heading", { name: "Jelly Bunny", exact: true })
        .waitFor();
      await page.locator(".feed-status .status.live").waitFor();
      assert.equal(
        await page.locator('.collection-tile[data-collection-id=""]').count(),
        0,
      );
      assert.equal(await button("Model").isDisabled(), false);
      const params = new URL(page.url()).searchParams;
      assert.ok(params.get("f")?.startsWith("c=bunny"));
      if (search.includes("Raspberry")) {
        assert.ok(params.get("f").includes("b=@name:Raspberry"));
        assert.equal(params.get("mode"), "executed");
      }
    }
  }
  assert.deepEqual(errors, []);
  console.log(
    "Orderbook browser passed: unlimited trait previews, infinite scroll, Time/Gift #/Price sorting, currencies, URL restore, modes, live history/backfills with scroll anchors, accessible details and automatic Mini App catalog updates.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
