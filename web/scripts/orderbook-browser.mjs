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
const option = (id, name, attribute_id, meta = {}) => ({
  kind: "attribute_option",
  id,
  name,
  attribute_id,
  meta,
  aliases: [],
});
const models = [
  option("barbie", "Barbie", "tg.model"),
  option("juicy", "Juicy Mint", "tg.model"),
  option("grape", "Grape Glaze", "tg.model"),
  option("ivory", "Ivory", "tg.model"),
  option("rose", "Rose", "tg.model"),
];
const backdrops = [
  option("raspberry", "Raspberry", "tg.backdrop", {
    "colors.center_color": "#e07b85",
    "colors.edge_color": "#b65980",
    "colors.pattern_color": "#783950",
  }),
  option("hunter", "Hunter Green", "tg.backdrop", {
    "colors.center_color": "#8fae78",
    "colors.edge_color": "#4b825b",
    "colors.pattern_color": "#285038",
  }),
];
backdrops.push(
  ...["Midnight", "Amber", "Lavender"].map((name, index) =>
    option(`backdrop-${index}`, name, "tg.backdrop", {
      "colors.center_color": "#c493ab",
      "colors.edge_color": "#75506e",
      "colors.pattern_color": `#${["563447", "254829", "633553"][index]}`,
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
].map((name) => option(name.toLowerCase(), name, "tg.pattern"));
const collections = ["Jelly Bunny", "Plush Pepe"].map((name, index) => ({
  kind: "item_type",
  id: index ? "pepe" : "bunny",
  name,
  quantity_step: "1",
  aliases: [],
  attributes: {
    "tg.model": { required: true, allowed_values: models.map((o) => o.id) },
    "tg.backdrop": {
      required: true,
      allowed_values: backdrops.map((o) => o.id),
    },
    "tg.pattern": { required: true, allowed_values: patterns.map((o) => o.id) },
    "tg.number": { required: true },
  },
}));
const venues = ["Telegram", "Portals", "MRKT"].map((name) => ({
  kind: "venue",
  id: name.toLowerCase(),
  name,
}));
const attributes = ["model", "backdrop", "pattern", "number"].map((name) => ({
  kind: "attribute",
  id: `tg.${name}`,
  name: name[0].toUpperCase() + name.slice(1),
  value_type: name === "number" ? "integer" : "enum",
  fields: {},
  element: null,
}));
const entities = [
  ...collections,
  ...venues,
  ...attributes,
  ...models,
  ...backdrops,
  ...patterns,
];
const eq = (type, value) => ({ op: "eq", value: { type, value } });
let revision = 0;
const sync = { ready: true, state: "live", updated_at: "2026-11-17T00:00:00Z" };
function result(query, executed = false) {
  const scope = query.scope;
  const mode = scope.modes[0];
  let rows = Array.from(
    { length: executed ? 75 : mode === "bid" ? 4 : 125 },
    (_, index) => ({
      id: `order-${index}`,
      item_id: `gift-${1000 + index}`,
      book: {
        item_type_id: scope.item_type_id,
        currency_id: scope.currency_id,
        venue_id: venues[index % 3].id,
      },
      mode,
      quantity: "1",
      price:
        scope.currency_id === "XTR"
          ? String(850 + index)
          : index === 0
            ? revision
              ? "0.250000001"
              : "0.500000001"
            : index === 59
              ? "10000000000000000.000000001"
              : String(index + 10),
      attributes: {
        "tg.model": eq("enum", models[index % models.length].id),
        "tg.backdrop": eq("enum", backdrops[index % backdrops.length].id),
        "tg.pattern": eq("enum", patterns[index % patterns.length].id),
        "tg.number": eq("integer", String(2000 - index)),
      },
      observed_at: new Date(1700000000000 + index * 1000).toISOString(),
      executed_at: executed
        ? new Date(1700000000000 + index * 1000).toISOString()
        : null,
      parent_order_id: null,
      meta: { listing_url: "https://t.me/market/app?startapp=gift42" },
    }),
  );
  rows = rows.filter(
    (order) =>
      (!scope.venue_id || scope.venue_id === order.book.venue_id) &&
      Object.entries(scope.attributes || {}).every(([id, p]) =>
        p.op === "eq"
          ? order.attributes[id]?.value.value === p.value.value
          : p.op === "one_of"
            ? p.values.some(
                (value) => value.value === order.attributes[id]?.value.value,
              )
            : true,
      ) &&
      (!scope.price?.max || Number(order.price) <= Number(scope.price.max)),
  );
  rows.sort((a, b) => {
    const rank = (order) =>
      query.sort?.startsWith("time_")
        ? Date.parse(executed ? order.executed_at : order.observed_at)
        : query.sort?.startsWith("number_")
          ? Number(order.attributes["tg.number"].value.value)
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
  let failOptions = true,
    optionRequests = 0;
  await page.route("**/catalog/query", (route) => {
    const body = route.request().postDataJSON();
    if (body.kind === "attribute_option") {
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
    }
    return route.fulfill({
      json: {
        data: entities.filter(
          (row) =>
            row.kind === body.kind &&
            (!body.ids?.length || body.ids.includes(row.id)),
        ),
        next_page_token: "",
        cursor: "fixture",
      },
    });
  });
  await page.route("**/books?*", (route) =>
    route.fulfill({
      json: {
        data: collections.flatMap((item) =>
          ["GRAM", "XTR"].flatMap((currency) =>
            venues.map((venue) => ({
              item_type_id: item.id,
              currency_id: currency,
              venue_id: venue.id,
            })),
          ),
        ),
        next_after: null,
      },
    }),
  );
  for (const path of ["orders", "executions"])
    await page.route(`**/${path}/query`, (route) =>
      route.fulfill({
        json: result(route.request().postDataJSON(), path === "executions"),
      }),
    );
  const connections = new Set();
  await page.routeWebSocket("**/ws", (socket) => {
    let connection;
    socket.onMessage((message) => {
      const frame = JSON.parse(String(message));
      if (frame.type === "subscribe") {
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
    socket.onClose(() => connections.delete(connection));
  });
  const base = `http://127.0.0.1:${server.httpServer.address().port}`;
  await page.goto(`${base}/?item=bunny&currency=GRAM&sort=price_asc`);
  const button = (name) => page.getByRole("button", { name, exact: true });
  const cards = (n) =>
    page.waitForFunction(
      (n) => document.querySelectorAll(".gift-card").length === n,
      n,
    );
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
    await page.locator(".scroll-loader").scrollIntoViewIfNeeded();
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
    125,
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
  await page.waitForURL(/item=pepe/);
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
  await page.goto(`${base}/?item=bunny&currency=GRAM&sort=price_asc&max=12`);
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
  await button("History").click();
  await cards(50);
  await page.getByText("Snapshot", { exact: true }).waitFor();
  await scrollMore(75);
  await button("Asks").click();
  await cards(50);
  const opener = page.locator(".card-open").first();
  await opener.click();
  await page.getByRole("dialog", { name: "Order details" }).waitFor();
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
  assert.deepEqual(errors, []);
  console.log(
    "Orderbook browser passed: unlimited trait previews, infinite scroll, Time/Gift #/Price sorting, currencies, URL restore, modes and accessible details.",
  );
} finally {
  await browser?.close();
  await server.close();
}
