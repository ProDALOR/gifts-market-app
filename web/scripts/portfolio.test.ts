import assert from "node:assert/strict";
import { test } from "node:test";
import {
  compareValue,
  selectItems,
  progressText,
} from "../src/portfolio/model.ts";
import type {
  PortfolioFilters,
  PortfolioItem,
  PortfolioState,
} from "../src/portfolio/model.ts";
import { parseFrame } from "../src/ws-protocol.ts";
const filters: PortfolioFilters = {
  collection: "",
  traits: {},
  number: "",
  sort: "value_desc",
};
const item = (id: number, estimate?: string): PortfolioItem => ({
  gift: {
    gift: {
      id: String(id),
      slug: `Gift-${id}`,
      number: id,
      collection: "1",
      model: "2",
    },
    relation: "owned",
  },
  resolved: {
    id: String(id),
    number: id,
    collection_id: "gift",
    traits: { model: "model", backdrop: "back", pattern: "symbol" },
  },
  estimate,
});
const state: PortfolioState = {
  user_id: 7,
  refresh_id: 1,
  progress: {
    stage: "ready",
    checked: 2,
    total: 2,
    pages: 1,
    valued: 2,
    collectibles: 2,
  },
  inventory: {
    checked_at: "2026-11-24T00:00:00Z",
    count: 2,
    scanned: 2,
    regular: 0,
    skipped: 0,
    complete: true,
  },
  valuation: {
    currency_id: "GRAM",
    evaluated_at: "2026-11-24T00:00:00Z",
    stale: false,
    total: "9999999999999999.01",
    priced: 1,
    unpriced: 1,
    items: [item(1, "9999999999999999.01"), item(2)],
  },
};
test("portfolio sorting retains exact decimals and keeps unknown prices last", () => {
  assert(compareValue("9999999999999999.02", "9999999999999999.01") > 0);
  assert.equal(compareValue("001.00", "1"), 0);
  const items = [
    item(1, "9999999999999999.01"),
    item(2, "9999999999999999.02"),
    item(3),
  ];
  assert.deepEqual(
    selectItems(items, filters).map((i) => i.gift.gift.number),
    [2, 1, 3],
  );
  assert.deepEqual(
    selectItems(items, { ...filters, sort: "value_asc" }).map(
      (i) => i.gift.gift.number,
    ),
    [1, 2, 3],
  );
  assert.equal(
    selectItems(items, { ...filters, traits: { models: ["missing"] } }).length,
    0,
  );
  assert.equal(
    selectItems(items, {
      ...filters,
      traits: { models: ["model"], backdrops: ["back"], patterns: ["symbol"] },
      number: "2",
    }).length,
    1,
  );
  assert.equal(
    items[0].gift.gift.number,
    1,
    "selection does not mutate cached order",
  );
});
test("portfolio progress handles unknown totals, partial inventory and retained failures", () => {
  assert.match(
    progressText({
      ...state,
      progress: {
        ...state.progress,
        stage: "loading_inventory",
        total: undefined,
        checked: 100,
        pages: 1,
      },
    }),
    /100 gifts checked/,
  );
  assert.equal(
    progressText({
      ...state,
      inventory: { ...state.inventory!, complete: false },
    }),
    "Partial inventory",
  );
  assert.equal(
    progressText({
      ...state,
      progress: { ...state.progress, stage: "failed" },
      error: "Previous inventory retained",
    }),
    "Previous inventory retained",
  );
});
test("portfolio wire frames reject invalid identities, progress and malformed consumed gift fields", () => {
  const frame = { type: "portfolio.snapshot", id: "p.1", data: state };
  assert.equal(parseFrame(JSON.stringify(frame)).type, "portfolio.snapshot");
  for (const change of [
    (s: PortfolioState) => {
      s.user_id = 0;
    },
    (s: PortfolioState) => {
      s.progress.checked = -1;
    },
    (s: PortfolioState) => {
      s.valuation!.items[0].gift.gift.number = 0;
    },
    (s: PortfolioState) => {
      s.valuation!.items[0].estimate = "not a price";
    },
  ]) {
    const bad = structuredClone(frame);
    change(bad.data);
    assert.throws(() => parseFrame(JSON.stringify(bad)));
  }
  const progress = {
    type: "portfolio.progress",
    id: "p.1",
    refresh_id: 2,
    progress: { ...state.progress, stage: "queued" },
  };
  assert.equal(parseFrame(JSON.stringify(progress)).type, "portfolio.progress");
  assert.throws(() =>
    parseFrame(
      JSON.stringify({
        ...progress,
        progress: { ...progress.progress, stage: "unknown" },
      }),
    ),
  );
  assert.equal(
    parseFrame(
      JSON.stringify({
        type: "result",
        id: "q.1",
        method: "portfolio.query",
        data: state,
      }),
    ).type,
    "result",
  );
});

test("selection summaries keep unknown estimates distinct from empty portfolios", async () => {
  const { portfolioSummary } = await import("../src/portfolio/model.ts");
  assert.equal(
    portfolioSummary([item(1, "1.23"), item(2, "2.34"), item(3)]).total,
    "3.57",
  );
  assert.equal(portfolioSummary([item(1)]).total, undefined);
  assert.equal(portfolioSummary([]).total, "0");
  assert.equal(portfolioSummary([item(1), item(2)]).unpriced, 2);
});

test("visible unverified gifts remain filterable and counted without an ownership claim", async () => {
  const { portfolioSummary, portfolioRelation } =
    await import("../src/portfolio/model.ts");
  const minted = item(2, "2.34");
  minted.gift.relation = "unverified";
  const hosted = item(3, "3.45");
  hosted.gift.relation = "hosted";
  const items = [item(1, "1.23"), minted, hosted];
  assert.deepEqual(
    selectItems(items, filters).map((i) => i.gift.gift.number),
    [3, 2, 1],
  );
  assert.equal(selectItems(items, { ...filters, number: "2" })[0], minted);
  assert.equal(portfolioSummary(items).total, "7.02");
  assert.equal(portfolioSummary(items).unverified, 1);
  assert.equal(portfolioRelation("owned"), undefined);
  assert.equal(portfolioRelation("hosted")?.label, "Hosted");
  assert.equal(
    portfolioRelation("unverified")?.description,
    "Visible on this profile; ownership unverified.",
  );
  const frame = {
    type: "portfolio.snapshot",
    id: "p.1",
    data: {
      ...state,
      valuation: {
        ...state.valuation!,
        items,
        priced: 3,
        unpriced: 0,
        total: "7.02",
      },
    },
  };
  assert.equal(parseFrame(JSON.stringify(frame)).type, "portfolio.snapshot");
});

test("position sorting follows source order, including filtered and unpriced gifts", () => {
  const source = [item(90, "1"), item(3), item(25, "9"), item(1, "2")];
  assert.deepEqual(
    selectItems(source, { ...filters, sort: "position_asc" }).map(
      (i) => i.gift.gift.number,
    ),
    [90, 3, 25, 1],
  );
  assert.deepEqual(
    selectItems(source, { ...filters, sort: "position_desc" }).map(
      (i) => i.gift.gift.number,
    ),
    [1, 25, 3, 90],
  );
  assert.deepEqual(
    source.map((i) => i.gift.gift.number),
    [90, 3, 25, 1],
  );
  assert.equal(
    selectItems(source, { ...filters, sort: "position_asc", number: "3" })[0],
    source[1],
  );
});

test("portfolio filters keep profile order across collection and trait changes", () => {
  const source = [item(90, "1"), item(3), item(25, "9"), item(1, "2")];
  source[1].resolved!.collection_id = "other";
  source[2].resolved!.traits.model = "second-model";
  source[3].resolved!.traits.backdrop = "second-backdrop";
  const position = { ...filters, sort: "position_asc" as const };
  const numbers = (f: PortfolioFilters, items = source) =>
    selectItems(items, f).map((i) => i.gift.gift.number);
  assert.deepEqual(numbers({ ...position, collection: "gift" }), [90, 25, 1]);
  assert.deepEqual(
    numbers({
      ...position,
      traits: { models: ["model"], backdrops: ["back"] },
    }),
    [90, 3],
  );
  assert.deepEqual(
    numbers({
      ...position,
      collection: "gift",
      traits: { models: ["second-model", "model"], patterns: ["symbol"] },
      sort: "position_desc",
    }),
    [1, 25, 90],
  );
  assert.deepEqual(
    numbers(position),
    [90, 3, 25, 1],
    "All gifts restores the source order",
  );
  assert.deepEqual(
    numbers(position, [source[3], source[1], source[0]]),
    [1, 3, 90],
    "a refreshed inventory supplies the new profile order",
  );
});
