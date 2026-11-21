import { collectionOptions } from "../src/catalog/selectors.ts";
import assert from "node:assert/strict";
import { test } from "node:test";
import { listingLink, serial, modelName } from "../src/orderbook.ts";
import type { Collection, CatalogOption, Order } from "../src/market.ts";
test("concrete gift numbers remain distinct from bid selection ranges", () => {
  const gift = {
    target: { kind: "gift", number: 42, traits: { model: "gold" } },
  } as Order;
  const bid = {
    target: {
      kind: "criteria",
      models: ["gold"],
      number: { min: 1, max: 100 },
    },
  } as Order;
  assert.equal(serial(gift), "42");
  assert.equal(serial(bid), undefined);
  assert.equal(
    modelName(
      bid,
      new Map([
        ["gold", { id: "gold", name: "Gold", kind: "model", media: null }],
      ]),
      "Any",
    ),
    "Gold",
  );
});
test("trait choices use the active collection response and matching gift trait", () => {
  const collection: Collection = {
    id: "gift",
    name: "Gift",
    slug: "Gift",
    preview: null,
  };
  const options = new Map<string, CatalogOption>([
    ["gold", { id: "gold", name: "Gold", kind: "model", media: null }],
    ["silver", { id: "silver", name: "Silver", kind: "model", media: null }],
  ]);
  assert.deepEqual(
    collectionOptions(collection, "models", options).map((option) => option.id),
    ["gold", "silver"],
  );
  assert.deepEqual(collectionOptions(collection, "patterns", options), []);
});
test("market buttons require a public HTTP(S) source link", () => {
  for (const market_url of [
    "javascript:alert(1)",
    "data:text/html,bad",
    "https://user:secret@example.com",
    "not a url",
  ]) {
    assert.equal(listingLink({ market_url } as Order), undefined);
  }
  assert.equal(
    listingLink({
      market_url: "https://t.me/market/app?startapp=gift42",
    } as Order),
    "https://t.me/market/app?startapp=gift42",
  );
});

test("gift details show traits and bid constraints without listing metadata", async () => {
  const { giftDetails } = await import("../src/orderbook.ts");
  const base: Order = {
    id: "private-source-listing",
    book: { venue_id: "venue", collection_id: "gift", currency_id: "GRAM" },
    kind: "ask",
    price: "1",
    quantity: 1,
    observed_at: "2026-11-18T12:00:00Z",
    executed_at: null,
    expires_at: "2026-11-19T12:00:00Z",
    market_url: null,
    target: {
      kind: "gift",
      id: "source-gift-id",
      number: 42,
      traits: { model: "model", backdrop: null, pattern: null },
    },
  };
  const names = new Map([
    ["venue", "Market"],
    ["model", "Gold"],
  ]);
  const rows = giftDetails(base, names),
    labels = rows.map(({ label }) => label);
  assert.ok(!labels.includes("Last observed"));
  assert.ok(!labels.includes("Expires"));
  assert.ok(!labels.includes("Quantity"));
  for (const label of [
    "Mode",
    "Order ID",
    "Gift ID",
    "Parent order",
    "Currency",
    "Gift #",
  ])
    assert.ok(!labels.includes(label));
  assert.ok(!JSON.stringify(rows).includes("source-"));
  assert.deepEqual(
    rows.find(({ label }) => label === "Model"),
    { label: "Model", value: "Gold", attribute: "model" },
  );
  assert.ok(!labels.includes("Backdrop"));

  const bid: Order = {
    ...base,
    kind: "bid",
    quantity: 5,
    target: {
      kind: "criteria",
      models: [],
      backdrops: [],
      patterns: [],
      number: { min: 1, max: 10 },
    },
  };
  const bidRows = giftDetails(bid, names);
  assert.equal(
    bidRows.some(({ label }) => label === "Quantity"),
    false,
    "Quantity belongs to the artwork badge",
  );
  assert.deepEqual(
    bidRows.find(({ label }) => label === "Model"),
    { label: "Model", value: "Any", attribute: "model" },
  );
  assert.deepEqual(
    bidRows.find(({ label }) => label === "Gift #"),
    { label: "Gift #", value: "1 – 10" },
  );
  const executedRows = giftDetails(
    { ...base, kind: "executed", executed_at: "2026-11-18T13:00:00Z" },
    names,
  );
  assert.ok(!executedRows.some(({ label }) => label === "Executed"));
  assert.ok(
    !executedRows.some(
      ({ label }) => label === "Expires" || label === "Last observed",
    ),
  );
});

test("unsupported conditions remain distinct from unrestricted criteria in cards and details", async () => {
  const { giftDetails } = await import("../src/orderbook.ts");
  const order = {
    book: { venue_id: "venue" },
    kind: "bid",
    quantity: 1,
    observed_at: "2026-11-18T12:00:00Z",
    target: {
      kind: "unsupported",
      code: "unsupported_constraints",
      message: "Some conditions cannot be displayed yet.",
    },
  } as Order;
  assert.equal(serial(order), undefined);
  assert.equal(modelName(order, new Map(), "Any"), "Unsupported conditions");
  const rows = giftDetails(order, new Map());
  assert.deepEqual(rows[0], {
    label: "Conditions",
    value: order.target.kind === "unsupported" && order.target.message,
  });
  assert.ok(!rows.some(({ value }) => value === "Any"));
});

test("detail visuals resolve with their labels and only for a single matching trait", async () => {
  const { giftDetails } = await import("../src/orderbook.ts");
  const model: CatalogOption = {
    id: "model",
    name: "Gold",
    kind: "model",
    media: null,
  };
  const backdrop: CatalogOption = {
    id: "backdrop",
    name: "Blue",
    kind: "backdrop",
    palette: null,
  };
  const options = new Map([
    [model.id, model],
    [backdrop.id, backdrop],
  ]);
  const names = new Map([
    [model.id, model.name],
    [backdrop.id, backdrop.name],
  ]);
  const gift = {
    target: {
      kind: "gift",
      traits: { model: "model", backdrop: "backdrop", pattern: null },
    },
  } as Order;
  assert.deepEqual(giftDetails(gift, names, options), [
    { label: "Model", value: "Gold", attribute: "model", option: model },
    {
      label: "Backdrop",
      value: "Blue",
      attribute: "backdrop",
      option: backdrop,
    },
  ]);
  const bid = {
    quantity: 1,
    target: {
      kind: "criteria",
      models: ["model", "other"],
      backdrops: ["model"],
      patterns: [],
      number: null,
    },
  } as Order;
  assert.ok(
    giftDetails(bid, names, options).every((row) => row.option === undefined),
    "Multiple choices, wrong trait kinds and unrestricted bids have no invented preview",
  );
});

test("ask and history details share catalog-first labels and reject mismatched trait kinds", async () => {
  const { giftDetails } = await import("../src/orderbook.ts");
  const options = new Map<string, CatalogOption>([
    [
      "model",
      { id: "model", name: "Updated model", kind: "model", media: null },
    ],
    [
      "wrong",
      { id: "wrong", name: "Not a model", kind: "backdrop", palette: null },
    ],
  ]);
  const names = new Map([
    ["model", "Stale model"],
    ["wrong", "Stale alias"],
  ]);
  for (const kind of ["ask", "executed"] as const) {
    const gift = {
      kind,
      target: { kind: "gift", traits: { model: "model" } },
    } as Order;
    assert.equal(giftDetails(gift, names, options)[0].value, "Updated model");
    const invalid = {
      ...gift,
      target: { ...gift.target, kind: "gift", traits: { model: "wrong" } },
    } as Order;
    assert.equal(
      giftDetails(invalid, names, options)[0].value,
      "Unavailable model",
    );
    assert.equal(giftDetails(invalid, names, options)[0].option, undefined);
    assert.equal(modelName(invalid, options, "Missing"), "Missing");
  }
});
