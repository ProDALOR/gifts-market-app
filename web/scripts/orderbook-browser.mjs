// Full UI workflow with an isolated local catalog/feed; no Telegram or hub login.
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const server = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
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
          thumbnail_url: `/media/${kind === "pattern" ? "patterns" : "models"}/${id}`,
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
  preview: models[0].media,
  model_ids: models.map((o) => o.id),
  backdrop_ids: backdrops.map((o) => o.id),
  pattern_ids: patterns.map((o) => o.id),
}));
const venues = ["Telegram", "Portals", "MRKT"].map((name) => ({
  id: name.toLowerCase(),
  name,
}));
let revision = 0;
const sync = { ready: true, state: "live", updated_at: "2026-11-17T00:00:00Z" };
function result(query, executed = false) {
  const filters = query.filters;
  const mode = executed ? "executed" : query.side || "ask";
  let rows = Array.from(
    { length: executed ? 75 : mode === "bid" ? 4 : 125 },
    (_, index) => ({
      id: `order-${index}`,
      book: {
        collection_id: filters.collection_id,
        currency_id: filters.currency_id,
        venue_id: venues[index % 3].id,
      },
      kind: mode,
      quantity: mode === "bid" && index === 1 ? 5 : 1,
      price:
        filters.currency_id === "XTR"
          ? String(850 + index)
          : index === 0
            ? revision
              ? "0.250000001"
              : "0.500000001"
            : index === 59
              ? "10000000000000000.000000001"
              : String(index + 10),
      target:
        mode === "bid" && index === 0
          ? {
              kind: "unsupported",
              reason: {
                code: "unsupported_constraints",
                message:
                  "This listing has conditions that cannot be displayed yet.",
              },
            }
          : mode === "bid"
            ? {
                kind: "criteria",
                criteria: {
                  models: [],
                  backdrops: [],
                  patterns: [],
                  number: null,
                },
              }
            : {
                kind: "gift",
                gift: {
                  id: `gift-${1000 + index}`,
                  collection_id: filters.collection_id,
                  number: 2000 - index,
                  traits: {
                    model: models[index % models.length].id,
                    backdrop: backdrops[index % backdrops.length].id,
                    pattern: patterns[index % patterns.length].id,
                  },
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
  rows = rows.filter(
    (order) =>
      (!filters.venue_id || filters.venue_id === order.book.venue_id) &&
      ["models", "backdrops", "patterns"].every(
        (key) =>
          !filters[key]?.length ||
          (order.target.kind === "gift" &&
            filters[key].includes(
              order.target.gift.traits[
                { models: "model", backdrops: "backdrop", patterns: "pattern" }[
                  key
                ]
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
            ? order.target.gift.number
            : 0
          : Number(order.price);
    return (rank(a) - rank(b)) * (query.sort?.endsWith("_desc") ? -1 : 1);
  });
  const start = Number(query.page_token || 0),
    limit = query.limit || 50;
  return {
    data: rows.slice(start, start + limit),
    next_page_token: start + limit < rows.length ? String(start + limit) : "",
    cursor: "fixture",
    sync,
  };
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
  const collectionRevisions = { bunny: "bunny.0", pepe: "pepe.0" };
  let catalogRequests = 0;
  let failOptions = true,
    optionRequests = 0;
  await page.route("**/catalog", (route) => {
    catalogRequests++;
    return route.fulfill({
      headers: { etag: '"directory.0"' },
      json: {
        collections,
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
  for (const path of ["orders", "history"])
    await page.route(`**/${path}/query`, (route) =>
      route.fulfill({
        json: result(route.request().postDataJSON(), path === "history"),
      }),
    );
  const connections = new Set();
  const catalogPeers = new Set();
  const openSockets = new Set();
  let socketCount = 0,
    orderSubscriptions = 0;
  const sendCatalog = (peer) =>
    peer.socket.send(
      JSON.stringify({
        type: "catalog",
        id: "catalog",
        versions: {
          directory: "directory.0",
          collections: Object.fromEntries(
            peer.ids.map((id) => [id, collectionRevisions[id] ?? null]),
          ),
        },
      }),
    );
  const notifyCatalog = () => {
    for (const peer of catalogPeers) sendCatalog(peer);
  };
  await page.routeWebSocket("**/ws", (socket) => {
    socketCount++;
    openSockets.add(socket);
    let connection, catalogPeer;
    socket.onMessage((message) => {
      const frame = JSON.parse(String(message));
      if (frame.type === "unsubscribe") {
        if (frame.id === "offers") connections.delete(connection);
        if (frame.id === "catalog") catalogPeers.delete(catalogPeer);
      }
      if (frame.type === "watch_catalog") {
        catalogPeers.delete(catalogPeer);
        catalogPeer = { socket, ids: frame.collection_ids };
        catalogPeers.add(catalogPeer);
        sendCatalog(catalogPeer);
      }
      if (frame.type === "subscribe") {
        orderSubscriptions++;
        connections.delete(connection);
        connection = { socket, query: frame.query };
        connections.add(connection);
        socket.send(
          JSON.stringify({
            type: "page",
            id: frame.id,
            page: result(frame.query),
          }),
        );
      }
    });
    socket.onClose(() => {
      connections.delete(connection);
      catalogPeers.delete(catalogPeer);
      openSockets.delete(socket);
    });
  });
  const base = `http://127.0.0.1:${server.httpServer.address().port}`;
  await page.goto(`${base}/?collection=bunny&currency=GRAM&sort=price_asc`);
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
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    await cards(count);
  };
  await cards(50);
  await price("0.500000001");
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
    await page.locator(".collection-tile").evaluateAll((tiles) =>
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
  await cards(50);
  await price("0.500000001");
  await scrollMore(100);
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
  await button("Ⅱ Pause").click();
  await page.getByText("Paused", { exact: true }).waitFor();
  await cards(125);
  await button("▶ Resume").click();
  await page.getByText("Live", { exact: true }).waitFor();
  await cards(125);
  await button("Search collections").click();
  await page.getByLabel("Filter collections").fill("pepe");
  assert.equal(await page.locator(".collection-tile").count(), 1);
  await page.getByLabel("Filter collections").press("Enter");
  await page.waitForURL(/collection=pepe/);
  await cards(50);
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
  await cards(50);
  assert.equal(
    optionRequests,
    3,
    "Returning to a loaded collection reuses its complete metadata",
  );
  // Every selected trait must appear in the composition, not only the first three.
  await button("Model").click();
  for (const model of models) await button(model.name).click();
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
  await cards(50);
  await button("Reset traits").click();
  await cards(50);
  await button("Model").click();
  await button("Barbie").click();
  await cards(25);
  await page.keyboard.press("Escape");
  await page.reload();
  await cards(25);
  assert.ok(page.url().includes("filters="));
  await button("Reset traits").click();
  await cards(50);
  await button("Telegram").click();
  await cards(42);
  await button("All venues").click();
  await cards(50);
  await button("Reverse sort direction").click();
  await price("10,000,000,000,000,000.000000001");
  for (const width of [320, 390, 480, 1440]) {
    await page.setViewportSize({ width, height: 844 });
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
      false,
      `No overflow at ${width}`,
    );
    assert.equal(
      await page
        .locator(".gift-grid")
        .evaluate(
          (el) => getComputedStyle(el).gridTemplateColumns.split(" ").length,
        ),
      3,
    );
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await sort("Gift #");
  await page.waitForFunction(
    () => document.querySelector(".gift-card .serial")?.textContent === "#2000",
  );
  await sort("Time");
  await page.waitForFunction(
    () => document.querySelector(".gift-card .serial")?.textContent === "#1876",
  );
  await sort("Price");
  await button("Reverse sort direction").click();
  await price("0.500000001");
  await currency("XTR");
  await price("850");
  assert.equal(
    await page.locator(".gift-card").first().locator(".star").count(),
    1,
  );
  await currency("GRAM");
  await price("0.500000001");
  await page.goto(
    `${base}/?collection=bunny&currency=GRAM&sort=price_asc&max=12`,
  );
  await cards(3);
  await currency("XTR");
  await cards(50);
  assert.equal(
    new URL(page.url()).searchParams.has("max"),
    false,
    "Changing units clears a bound denominated in the old currency",
  );
  await currency("GRAM");
  await price("0.500000001");
  await button("Ⅱ Pause").click();
  await page.getByText("Paused", { exact: true }).waitFor();
  revision = 1;
  for (const { socket, query } of connections)
    socket.send(
      JSON.stringify({ type: "page", id: "offers", page: result(query) }),
    );
  await price("0.500000001");
  await button("▶ Resume").click();
  await price("0.250000001");
  await button("Bids").click();
  await cards(4);
  assert.equal(
    await page
      .locator(".app-footer")
      .evaluate(
        (footer) =>
          Math.abs(
            footer.getBoundingClientRect().bottom -
              (innerHeight -
                parseFloat(
                  getComputedStyle(footer.parentElement).paddingBottom,
                )),
          ) < 2,
      ),
    true,
    "Short books keep the footer at the viewport bottom",
  );
  assert.equal(await button("Model").isDisabled(), true);
  assert.equal(
    await page
      .locator(".gift-card")
      .first()
      .getByText("Unsupported conditions", { exact: true })
      .count(),
    1,
  );
  await page.locator(".card-open").first().click();
  await page
    .locator(".detail")
    .getByText("This listing has conditions that cannot be displayed yet.", {
      exact: true,
    })
    .waitFor();
  assert.equal(await page.locator(".detail-market").count(), 0);
  assert.equal(
    await page
      .locator(".detail dt")
      .getByText("Model", { exact: true })
      .count(),
    0,
  );
  await page.keyboard.press("Escape");
  await page.locator(".card-open").nth(1).click();
  await page
    .locator(".detail dt")
    .getByText("Quantity", { exact: true })
    .waitFor();
  assert.equal(
    await page.locator(".detail dd").getByText("5", { exact: true }).count(),
    1,
  );
  await page.keyboard.press("Escape");
  await button("History").click();
  await cards(50);
  await page.getByText("Snapshot", { exact: true }).waitFor();
  await scrollMore(75);
  await page.locator(".card-open").first().click();
  await page
    .locator(".detail dt")
    .getByText("Executed", { exact: true })
    .waitFor();
  assert.equal(
    await page
      .locator(".detail dt")
      .getByText("Last observed", { exact: true })
      .count(),
    0,
  );
  await page.keyboard.press("Escape");
  await button("Asks").click();
  await cards(50);
  const opener = page.locator(".card-open").first();
  await opener.click();
  await page.getByRole("dialog", { name: "Order details" }).waitFor();
  await page
    .locator(".detail dt")
    .getByText("Last observed", { exact: true })
    .waitFor();
  const labels = await page.locator(".detail dt").allTextContents();
  for (const hidden of [
    "Order ID",
    "Gift ID",
    "Parent order",
    "Mode",
    "Quantity",
    "Gift #",
  ])
    assert.ok(!labels.includes(hidden), hidden);
  assert.equal(await page.locator(".source-details").count(), 0);
  assert.equal(
    await button("Close details").evaluate(
      (el) => el === document.activeElement,
    ),
    true,
  );
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
  await cards(50);
  await page.waitForFunction(() =>
    document.documentElement.classList.contains("telegram"),
  );
  const choose = async (name) => {
    await button("Search collections").click();
    await page.getByLabel("Filter collections").fill(name);
    await page.getByLabel("Filter collections").press("Enter");
    await cards(50);
  };
  await choose("pepe");
  await choose("bunny");
  await new Promise((resolve) => setTimeout(resolve, 100));
  let beforeRequests = optionRequests;
  const beforeCatalog = catalogRequests;
  collectionRevisions.pepe = "pepe.1";
  notifyCatalog();
  await new Promise((resolve) => setTimeout(resolve, 150));
  assert.equal(optionRequests, beforeRequests, "Inactive changes remain lazy");
  assert.equal(openSockets.size, 1, "Catalog and orders share one socket");
  await button("History").click();
  await cards(50);
  models[0].name = "Live model";
  collectionRevisions.bunny = "bunny.1";
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
  collectionRevisions.bunny = "bunny.2";
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
  await cards(50);
  const healthyOrders = orderSubscriptions,
    healthySockets = socketCount;
  const beforeRecovery = optionRequests;
  let peer = [...catalogPeers][0];
  peer.socket.send(
    JSON.stringify({
      type: "error",
      id: "catalog",
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
      id: "catalog",
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
  assert.deepEqual(errors, []);
  console.log(
    "Orderbook browser passed: unlimited trait previews, infinite scroll, Time/Gift #/Price sorting, currencies, URL restore, modes, accessible details and automatic Mini App catalog updates.",
  );
} finally {
  await browser?.close();
  await server.close();
}
