import assert from "node:assert/strict";
import { test } from "node:test";
import type { TestContext } from "node:test";
import { ApiFailure } from "../src/api-error.ts";
import type { ValuationQuery, MarketStatus } from "../src/market.ts";
import {
  FloorsController,
  traitFloor,
} from "../src/valuation/floors-controller.ts";
import { catalogFloors } from "../src/valuation/catalog-floors.ts";
import { parseFrame } from "../src/ws-protocol.ts";
import {
  giftBasisChanged,
  evaluatedAfter,
} from "../src/valuation/freshness.ts";
import type { ValuationResponse } from "../src/market.ts";
import type { FloorsEntry } from "../src/valuation/floors-controller.ts";
test("gift estimates become stale when prices, counts or catalog membership change", () => {
  const count = 3,
    price = { value: "10" };
  const report: ValuationResponse = {
    target: {
      kind: "gift",
      gift: {
        id: "1",
        collection_id: "gift",
        number: 1,
        traits: { model: "model", backdrop: "backdrop" },
      },
    },
    currency_id: "GRAM",
    evaluated_at: "2026-11-20T12:00:00Z",
    stale: false,
    results: [
      {
        strategy: "gift_floor",
        data: {
          collection_floor: { ...price, count },
          model_floor: { ...price, count },
          backdrop_floor: { ...price, count },
          model_backdrop_floor: { ...price, count },
          model_multiplier: price,
          backdrop_multiplier: price,
          model_premium: { value: "0" },
          backdrop_premium: { value: "0" },
          additive_estimate: price,
          basis: "pair_capped",
          final_mark: price,
        },
        revision: "r1",
      },
    ],
  };
  const floor = { ...price, count };
  const shared: Pick<FloorsEntry, "floor" | "traits"> = {
    floor,
    traits: {
      models: { model: floor },
      backdrops: { backdrop: floor },
      patterns: {},
      pair: { model_id: "model", backdrop_id: "backdrop", floor },
    },
  };
  assert.equal(giftBasisChanged(report, shared), false);
  assert.equal(
    giftBasisChanged(report, {
      ...shared,
      traits: {
        ...shared.traits!,
        patterns: { symbol: { value: "123", count } },
      },
    }),
    false,
    "Independent symbol floors do not invalidate the gift-floor formula",
  );
  assert.equal(
    giftBasisChanged(report, {
      ...shared,
      floor: { ...floor, value: "11" },
    }),
    true,
  );
  assert.equal(
    giftBasisChanged(report, {
      ...shared,
      floor: { ...floor, count: 4 },
    }),
    true,
  );
  const removed = structuredClone(shared);
  delete removed.traits!.models.model;
  assert.equal(giftBasisChanged(report, removed), true);
  const pair = structuredClone(shared);
  pair.traits!.pair!.floor.value = "12";
  assert.equal(giftBasisChanged(report, pair), true);
  assert.equal(
    giftBasisChanged(report, { floor }),
    false,
    "Awaiting a trait baseline does not invent a difference",
  );
  const unavailable = structuredClone(report),
    outcome = unavailable.results[0];
  if ("error" in outcome || outcome.strategy !== "gift_floor")
    throw new Error("Expected a gift floor result");
  outcome.data.model_backdrop_floor = {
    count: 0,
    error: {
      code: "not_enough_data",
      message: "No matching pair asks",
      dependency: "model_backdrop_floor",
      available_samples: 0,
    },
  };
  const current = structuredClone(shared);
  current.traits!.pair!.floor = {
    error: {
      available_samples: 0,
      dependency: "model_backdrop_floor",
      message: "No matching pair asks",
      code: "not_enough_data",
    },
    count: 0,
  };
  assert.equal(
    giftBasisChanged(unavailable, current),
    false,
    "Error property order does not change the floor basis",
  );
  current.traits!.pair!.floor.error.available_samples = 1;
  assert.equal(giftBasisChanged(unavailable, current), true);
  current.traits!.pair!.floor = { value: "10", count: 0 };
  assert.equal(giftBasisChanged(unavailable, current), true);
});
test("attribute valuations use only the matching scope and never invent a symbol or missing value", () => {
  const floor = (value: string, count: number) => ({
    value,
    count,
  });
  const entry: FloorsEntry = {
    floor: floor("8", 7),
    traits: {
      models: { model: floor("10", 2) },
      backdrops: { backdrop: floor("9", 1) },
      patterns: { symbol: floor("8.5", 3) },
    },
    stale: false,
    evaluatedAt: "2026-11-20T12:00:00Z",
    used: 1,
  };
  assert.equal(traitFloor(entry, "model", "model"), entry.traits!.models.model);
  assert.equal(
    traitFloor(entry, "backdrop", "backdrop"),
    entry.traits!.backdrops.backdrop,
  );
  assert.equal(traitFloor(entry, "model", "missing"), undefined);
  assert.equal(traitFloor(entry, "pattern", "model"), undefined);
  assert.equal(
    traitFloor(entry, "pattern", "symbol"),
    entry.traits!.patterns!.symbol,
  );
  assert.equal(traitFloor(undefined, "model", "model"), undefined);
});

function report(
  query: ValuationQuery,
  value = "3",
  time = "2026-11-20T12:00:00Z",
): ValuationResponse {
  const scope = target(query),
    floor = { value, count: 3 };
  return {
    target: structuredClone(scope),
    currency_id: query.currency_id,
    stale: false,
    evaluated_at: time,
    results: [
      {
        strategy: "catalog_floor",
        revision: "r1",
        data: {
          collections: scope.collection_ids.map((collection_id) => ({
            collection_id,
            evaluated_at: time,
            floor,
            ...(scope.details?.collection_id === collection_id
              ? {
                  traits: {
                    models: { model: floor },
                    backdrops: { backdrop: floor },
                    patterns: {},
                    ...(scope.details.pair
                      ? { pair: { ...scope.details.pair, floor } }
                      : {}),
                  },
                }
              : {}),
          })),
        },
      },
    ],
  };
}
function target(query: ValuationQuery) {
  if (query.target.kind !== "catalog") throw Error();
  return query.target;
}
async function flush(t: TestContext, ms = 40) {
  t.mock.timers.tick(ms);
  for (let i = 0; i < 100; i++) await Promise.resolve();
}
function fixture(t: TestContext) {
  t.mock.timers.enable({
    apis: ["setTimeout", "setInterval", "Date"],
    now: 1_000_000,
  });
  const calls: { query: ValuationQuery; signal: AbortSignal }[] = [];
  let changed!: (status: MarketStatus) => void;
  let load = async (query: ValuationQuery, _signal: AbortSignal) =>
    report(query);
  const controller = new FloorsController({
    query: (query, signal) => {
      calls.push({ query, signal: signal! });
      return load(query, signal!);
    },
    watchStatus: (callback) => {
      changed = callback;
      callback("available");
      return () => {};
    },
  });
  t.after(() => controller.close());
  return {
    controller,
    calls,
    status: (value: MarketStatus) => changed(value),
    load: (next: typeof load) => {
      load = next;
    },
  };
}
test("startup fetches every collection in summary-only batches, including more than 128", async (t) => {
  const f = fixture(t),
    ids = Array.from({ length: 145 }, (_, i) => `gift-${i}`);
  f.controller.configure("GRAM", ids, "", null);
  await flush(t);
  assert.equal(f.calls.length, 19);
  assert.ok(
    f.calls.every(
      ({ query }) =>
        !target(query).details && target(query).collection_ids.length <= 8,
    ),
  );
  assert.equal(f.controller.getSnapshot().entries.size, 145);
  f.controller.configure("GRAM", ids, "gift-4", null);
  await flush(t);
  assert.equal(f.calls.length, 20);
  assert.equal(target(f.calls.at(-1)!.query).details?.collection_id, "gift-4");
  assert.ok(f.controller.getSnapshot().entries.get("gift-4")?.traits);
});
test("collections and selected traits refresh every minute without polling on rerenders", async (t) => {
  const f = fixture(t),
    ids = ["a", "b", "c"];
  f.controller.configure("GRAM", ids, "a", null);
  await flush(t);
  assert.equal(f.calls.length, 2);
  f.controller.configure("GRAM", [...ids], "a", null);
  await flush(t, 59_000);
  assert.equal(f.calls.length, 2);
  await flush(t, 1_000);
  await flush(t);
  assert.equal(f.calls.length, 4);
  assert.equal(f.calls.filter((c) => target(c.query).details).length, 2);
  f.controller.configure("GRAM", ids, "b", null);
  await flush(t);
  assert.equal(f.calls.length, 5);
  f.controller.configure("GRAM", ids, "a", null);
  await flush(t);
  assert.equal(
    f.calls.length,
    5,
    "Returning within a minute reuses the validated trait cache",
  );
});
test("summary batches reuse trait payload weights without serializing cached maps again", async (t) => {
  const f = fixture(t);
  let serialized = 0;
  f.load(async (query) => {
    const result = report(query);
    const traits = catalogFloors(result.results[0].data)
      ? result.results[0].data.collections[0].traits
      : undefined;
    if (traits)
      Object.defineProperty(traits, "toJSON", {
        value() {
          serialized++;
          return { ...traits };
        },
      });
    return result;
  });
  const ids = Array.from({ length: 25 }, (_, i) => `gift-${i}`);
  f.controller.configure("GRAM", ids, "gift-0", null);
  await flush(t);
  assert.ok(f.controller.getSnapshot().entries.get("gift-0")?.traits);
  assert.equal(serialized, 1);
  f.controller.configure("GRAM", [...ids, "another"], "gift-0", null);
  await flush(t);
  assert.equal(f.controller.getSnapshot().entries.size, 26);
  assert.equal(
    serialized,
    1,
    "Unrelated summaries do not allocate new trait JSON strings",
  );
  await flush(t, 60_000);
  await flush(t);
  assert.equal(
    serialized,
    2,
    "A replacement trait payload gets a fresh weight",
  );
});
test("summary admission is bounded and selected details do not wait for the full sweep", async (t) => {
  const f = fixture(t),
    ids = Array.from({ length: 40 }, (_, i) => `gift-${i}`),
    releases: (() => void)[] = [];
  f.load(
    (q) => new Promise((resolve) => releases.push(() => resolve(report(q)))),
  );
  f.controller.configure("GRAM", ids, "gift-39", null);
  await flush(t);
  assert.equal(f.calls.length, 3);
  assert.equal(target(f.calls[0].query).details?.collection_id, "gift-39");
  await flush(t, 60_000);
  await flush(t);
  assert.equal(
    f.calls.length,
    3,
    "A slow minute refresh does not overlap in-flight jobs",
  );
  releases.forEach((release) => release());
  await flush(t, 0);
  assert.ok(f.calls.length > 3);
});
test("collection switches cancel old traits but keep the summary sweep; late responses cannot restore old pairs", async (t) => {
  const f = fixture(t);
  let release!: () => void;
  f.load((q) =>
    target(q).details
      ? new Promise((resolve) => {
          release = () => resolve(report(q, "999"));
        })
      : Promise.resolve(report(q)),
  );
  f.controller.configure("GRAM", ["a", "b"], "a", {
    model_id: "old",
    backdrop_id: "old",
  });
  await flush(t);
  const previous = f.calls.find((c) => target(c.query).details)!;
  f.load(async (q) => report(q));
  f.controller.configure("GRAM", ["a", "b"], "b", null);
  assert.equal(previous.signal.aborted, true);
  await flush(t);
  release();
  await flush(t, 0);
  assert.equal(f.controller.getSnapshot().entries.get("a")?.floor.value, "3");
  assert.ok(f.controller.getSnapshot().entries.get("b")?.traits);
  assert.equal(
    f.calls.filter((c) => !target(c.query).details).length,
    2,
    "Cancelled initial details are recovered as a summary so every collection is covered",
  );
});
test("currency changes and reconnects reject old results and refresh stale cached values", async (t) => {
  const f = fixture(t);
  f.controller.configure("GRAM", ["gift"], "gift", null);
  await flush(t);
  assert.equal(f.controller.getSnapshot().entries.get("gift")?.stale, false);
  f.status("unavailable");
  assert.equal(f.controller.getSnapshot().entries.get("gift")?.stale, true);
  const before = f.calls.length;
  await flush(t, 60_000);
  assert.equal(f.calls.length, before);
  f.status("available");
  await flush(t);
  assert.equal(f.calls.length, before + 1);
  assert.equal(f.controller.getSnapshot().entries.get("gift")?.stale, false);
  let release!: () => void;
  f.load(
    (q) =>
      new Promise((resolve) => {
        release = () => resolve(report(q, "999"));
      }),
  );
  f.controller.retry();
  await flush(t);
  const old = f.calls.at(-1)!;
  f.load(async (q) => report(q, "5"));
  f.controller.configure("XTR", ["gift"], "gift", null);
  assert.equal(f.controller.getSnapshot().entries.size, 0);
  assert.equal(old.signal.aborted, true);
  await flush(t);
  release();
  await flush(t, 0);
  assert.equal(f.controller.getSnapshot().currency, "XTR");
  assert.equal(
    f.controller.getSnapshot().entries.get("gift")?.floor.value,
    "5",
  );
});
test("oversized summary batches split while successful collections remain usable", async (t) => {
  const f = fixture(t);
  f.load(async (q) => {
    if (target(q).collection_ids.length > 2)
      throw new ApiFailure("capacity_exceeded", "batch too large");
    return report(q);
  });
  f.controller.configure(
    "GRAM",
    ["a", "b", "c", "d", "e", "f", "g", "h"],
    "",
    null,
  );
  await flush(t);
  assert.equal(f.controller.getSnapshot().entries.size, 8);
  assert.equal(f.controller.getSnapshot().error, "");
  assert.equal(f.calls.length, 7);
});
test("scope validation keeps existing prices stale, and close cancels work and periodic refresh", async (t) => {
  const f = fixture(t);
  f.controller.configure("GRAM", ["gift"], "gift", null);
  await flush(t);
  f.load(async (q) => ({ ...report(q), currency_id: "XTR" }));
  f.controller.retry();
  await flush(t);
  assert.match(f.controller.getSnapshot().error, /currency/);
  assert.equal(f.controller.getSnapshot().entries.get("gift")?.stale, true);
  const count = f.calls.length;
  f.controller.close();
  await flush(t, 180_000);
  assert.equal(f.calls.length, count);
});
test("nanosecond ordering rejects late summaries without discarding newer selected traits", async (t) => {
  const f = fixture(t);
  let release!: () => void;
  assert.equal(
    evaluatedAfter(
      "2026-11-20T12:00:00.123000001Z",
      "2026-11-20T12:00:00.123Z",
    ),
    true,
  );
  f.load((q) =>
    target(q).details
      ? Promise.resolve(report(q, "5", "2026-11-20T12:00:00.123000001Z"))
      : new Promise((resolve) => {
          release = () => resolve(report(q, "3", "2026-11-20T12:00:00.123Z"));
        }),
  );
  f.controller.configure("GRAM", ["gift"], "", null);
  await flush(t);
  f.controller.configure("GRAM", ["gift"], "gift", null);
  await flush(t);
  release();
  await flush(t, 0);
  assert.equal(
    f.controller.getSnapshot().entries.get("gift")?.floor.value,
    "5",
  );
  assert.ok(f.controller.getSnapshot().entries.get("gift")?.traits);
});
test("changing gift pairs clears the old pair before a deferred request finishes", async (t) => {
  const f = fixture(t);
  f.controller.configure("GRAM", ["gift"], "gift", {
    model_id: "old",
    backdrop_id: "old",
  });
  await flush(t);
  assert.equal(
    f.controller.getSnapshot().entries.get("gift")?.traits?.pair?.model_id,
    "old",
  );
  let release!: () => void;
  f.load(
    (q) =>
      new Promise((resolve) => {
        release = () => resolve(report(q));
      }),
  );
  f.controller.configure("GRAM", ["gift"], "gift", {
    model_id: "new",
    backdrop_id: "new",
  });
  assert.equal(
    f.controller.getSnapshot().entries.get("gift")?.traits?.pair,
    undefined,
  );
  await flush(t);
  release();
  await flush(t, 0);
  assert.equal(
    f.controller.getSnapshot().entries.get("gift")?.traits?.pair?.model_id,
    "new",
  );
});
test("all summaries are retained while trait maps remain bounded to eight selections", async (t) => {
  const f = fixture(t),
    ids = Array.from({ length: 12 }, (_, i) => `gift-${i}`);
  f.controller.configure("GRAM", ids, "", null);
  await flush(t);
  for (const id of ids) {
    f.controller.configure("GRAM", ids, id, null);
    await flush(t);
  }
  const entries = [...f.controller.getSnapshot().entries.values()];
  assert.equal(entries.length, 12);
  assert.equal(entries.filter((entry) => entry.traits).length, 8);
  assert.ok(f.controller.getSnapshot().entries.get(ids.at(-1)!)?.traits);
});

test("catalog validation rejects duplicate scopes and invalid counts and legacy floor wrappers", () => {
  const query: ValuationQuery = {
    target: { kind: "catalog", collection_ids: ["gift"] },
    currency_id: "GRAM",
    strategies: ["catalog_floor"],
  };
  const value = report(query),
    data = (value.results[0] as any).data;
  assert.equal(catalogFloors(data), true);
  data.collections[0].traits = {
    models: {},
    backdrops: {},
    patterns: { symbol: structuredClone(data.collections[0].floor) },
  };
  assert.equal(catalogFloors(data), true);
  data.collections[0].traits.patterns.symbol.count = -1;
  assert.equal(
    catalogFloors(data),
    false,
    "Symbol counts use the same validation as other traits",
  );
  delete data.collections[0].traits.patterns;
  assert.equal(
    catalogFloors(data),
    false,
    "The current API always supplies symbol floors",
  );
  const traits = {
    models: {},
    backdrops: {},
    patterns: { symbol: structuredClone(data.collections[0].floor) },
  };
  data.collections[0].traits = traits;
  assert.equal(
    catalogFloors(data),
    true,
    "Symbol floors use the same price/count validation",
  );
  traits.patterns.symbol.count = 1.5;
  assert.equal(
    catalogFloors(data),
    false,
    "Fractional symbol counts are rejected",
  );
  delete data.collections[0].traits;
  assert.ok(
    parseFrame(
      JSON.stringify({
        type: "result",
        id: "v.1",
        method: "valuation.query",
        data: value,
      }),
    ),
  );
  data.collections.push(data.collections[0]);
  assert.equal(catalogFloors(data), false);
  data.collections.pop();
  data.collections[0].floor.count = Number.MAX_SAFE_INTEGER + 1;
  assert.equal(catalogFloors(data), false);
});

test("selected floor loading and failure state remains scoped through retries and switching", async (t) => {
  const f = fixture(t);
  let release!: () => void;
  f.load((q) =>
    target(q).details
      ? new Promise((resolve) => (release = () => resolve(report(q))))
      : Promise.reject(new Error("Other collection failed")),
  );
  f.controller.configure("GRAM", ["a", "b"], "a", null);
  await flush(t);
  assert.deepEqual(f.controller.getSnapshot().detail, {
    pending: true,
    error: "",
  });
  release();
  await flush(t);
  assert.deepEqual(
    f.controller.getSnapshot().detail,
    { pending: false, error: "" },
    "A background collection error does not turn selected floors into failures",
  );
  f.load(async () => {
    throw new Error("Selected floors failed");
  });
  f.controller.retry();
  await flush(t);
  assert.deepEqual(f.controller.getSnapshot().detail, {
    pending: false,
    error: "Selected floors failed",
  });
  f.load(async (q) => report(q));
  f.controller.configure("GRAM", ["a", "b"], "b", null);
  await flush(t);
  assert.deepEqual(
    f.controller.getSnapshot().detail,
    { pending: false, error: "" },
    "Switching scopes clears the former error",
  );
});

test("older cached detail responses wait for the minute refresh instead of retrying in a loop", async (t) => {
  const f = fixture(t);
  f.load(async (q) => report(q, "5", "2026-11-20T12:01:00Z"));
  f.controller.configure("GRAM", ["gift"], "", null);
  await flush(t);
  let details = 0;
  f.load((q) => {
    if (target(q).details && ++details > 1) return new Promise(() => {}); // Bound the old retry-loop failure mode.
    return Promise.resolve(report(q, "1", "2026-11-20T12:00:00Z"));
  });
  f.controller.configure("GRAM", ["gift"], "gift", null);
  await flush(t);
  assert.equal(details, 1);
  assert.equal(
    f.controller.getSnapshot().entries.get("gift")?.floor.value,
    "5",
  );
  assert.equal(f.controller.getSnapshot().detail.pending, false);
  await flush(t, 1_000);
  assert.equal(details, 1);
  f.load(async (q) => report(q, "6", "2026-11-20T12:02:00Z"));
  await flush(t, 59_000);
  await flush(t);
  assert.equal(
    f.controller.getSnapshot().entries.get("gift")?.floor.value,
    "6",
  );
  assert.ok(f.controller.getSnapshot().entries.get("gift")?.traits);
});

test("a slow selected detail request owns its summary slot across refreshes and collection switches", async (t) => {
  const f = fixture(t),
    ids = Array.from({ length: 9 }, (_, i) => `gift-${i}`);
  let release!: () => void;
  f.load((q) =>
    target(q).details
      ? new Promise((resolve) => {
          release = () => resolve(report(q, "999"));
        })
      : Promise.resolve(report(q)),
  );
  f.controller.configure("GRAM", ids, "gift-0", null);
  await flush(t);
  await flush(t, 60_000);
  await flush(t);
  assert.ok(
    f.calls
      .filter(({ query }) => !target(query).details)
      .every(({ query }) => !target(query).collection_ids.includes("gift-0")),
    "The minute sweep cannot duplicate the in-flight detail's summary",
  );
  const old = f.calls.find(({ query }) => target(query).details)!;
  f.load(async (q) => report(q, "4"));
  f.controller.configure("GRAM", ids, "gift-1", null);
  assert.equal(old.signal.aborted, true);
  await flush(t);
  release();
  await flush(t, 0);
  assert.equal(
    f.controller.getSnapshot().entries.get("gift-0")?.floor.value,
    "4",
  );
  assert.equal(
    f.controller.getSnapshot().entries.get("gift-1")?.floor.value,
    "4",
  );
});
