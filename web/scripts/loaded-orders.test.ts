import assert from "node:assert/strict";
import { test } from "node:test";
import { LoadedOrders } from "../src/feed/loaded-orders.ts";
import type { components } from "../src/gen/api.ts";
type Window = components["schemas"]["OrderSnapshotResponse"];
const row = (id: number, rank = id): Window["data"][number] => ({
  id: String(id),
  key: `${String(rank * 1000).padStart(16, "0")}:${String(id).padStart(16, "0")}`,
  book: { venue_id: "v", collection_id: "gift", currency_id: "GRAM" },
  kind: "ask",
  quantity: 1,
  price: String(rank + 1),
  observed_at: "2026-11-18T12:00:00Z",
  target: {
    kind: "criteria",
    models: [],
    backdrops: [],
    patterns: [],
    number: null,
  },
});
const view = (data: Window["data"], before = true, after = true): Window => ({
  data,
  range: {
    start: before && data.length ? data[0].key : null,
    end: after && data.length ? data.at(-1)!.key + "~" : null,
  },
});
const range = (start: number, end: number) =>
  Array.from({ length: end - start }, (_, i) => row(start + i));
test("loaded regions survive forward and backward navigation with stable row objects", () => {
  const cache = new LoadedOrders();
  cache.merge(view(range(0, 150), false));
  const original = cache.rows[80];
  cache.merge(view(range(70, 220)));
  cache.merge(view(range(140, 290)));
  assert.equal(cache.rows.length, 290);
  cache.merge(view(range(0, 150), false));
  assert.equal(cache.rows.length, 290);
  assert.equal(cache.rows[80], original);
  assert.equal(cache.rows.at(-1)?.id, "289");
});
test("a refreshed window removes only missing rows inside its authoritative ordering range", () => {
  const cache = new LoadedOrders();
  cache.merge(view(range(0, 150), false));
  cache.merge(view(range(100, 250)));
  cache.merge(view(range(75, 175).filter((row) => row.id !== "120")));
  assert.equal(cache.rows.length, 249);
  assert.ok(!cache.rows.some((row) => row.id === "120"));
  assert.equal(cache.rows[0].id, "0");
  assert.equal(cache.rows.at(-1)?.id, "249");
});
test("a distant cached listing can be repriced into a window without erasing the intervening cache", () => {
  const cache = new LoadedOrders();
  for (let start = 0; start < 900; start += 100)
    cache.merge(view(range(start, start + 150), start !== 0));
  cache.merge(view([row(0), row(900, 0.5), ...range(1, 149)], false));
  assert.equal(cache.rows.length, 950);
  assert.equal(cache.rows[1].id, "900");
  assert.equal(cache.rows.filter((row) => row.id === "900").length, 1);
  assert.ok(cache.rows.some((row) => row.id === "600"));
});
test("revisiting the middle does not reopen a known end; an empty market clears cached rows", () => {
  const cache = new LoadedOrders();
  cache.merge(view(range(0, 150), false));
  cache.merge(view(range(100, 220), true, false));
  cache.merge(view(range(30, 180)));
  assert.equal(cache.rows.length, 220);
  cache.merge(view([], false, false));
  assert.equal(cache.rows.length, 0);
});
test("identical listing IDs at different venues stay distinct", () => {
  const cache = new LoadedOrders();
  const a = row(1),
    b = {
      ...row(1),
      book: { ...row(1).book, venue_id: "w" },
      key: `${a.key}w`,
    };
  cache.merge(view([a, b], false, false));
  cache.merge(view([{ ...a, price: "9" }, b], false, false));
  assert.equal(cache.rows.length, 2);
});

test("an empty bounded result clears its full coverage while retaining visited neighbors", () => {
  const cache = new LoadedOrders();
  const rows = range(0, 10);
  cache.merge(view(rows, false, false));
  cache.merge({
    data: [],
    range: {
      start: rows[3].key,
      end: rows[7].key,
    },
  });
  assert.deepEqual(
    cache.rows.map((row) => row.id),
    ["0", "1", "2", "7", "8", "9"],
  );
  assert.equal(cache.hasBefore, false);
});
