import assert from "node:assert/strict";
import { test } from "node:test";
import { WatchedOrders } from "../src/feed/watched-orders.ts";
const row = (id: string, key: string) =>
  ({
    id,
    key: key,
    book: { venue_id: "v", collection_id: "g", currency_id: "GRAM" },
  }) as any;
const snapshot = (data: any[], sequence = 0, reset = false) =>
  ({
    type: "orders.snapshot",
    sequence,
    reset,
    data,
    next: null,
    range: { start: null, end: "9" },
  }) as any;
const change = (base: number, upserts: any[] = [], removed: any[] = []) =>
  ({
    type: "orders.changed",
    base_sequence: base,
    sequence: base + 1,
    upserts,
    removed,
  }) as any;
test("range batches remove and reorder atomically without evicting unaffected rows", () => {
  const state = new WatchedOrders();
  const a = row("a", "1"),
    b = row("b", "2"),
    c = row("c", "3");
  state.apply(snapshot([a, b, c]));
  const next = state.apply(
    change(0, [row("b", "4")], [{ venue_id: "v", id: "a" }]),
  );
  assert.deepEqual(
    next.data.map((row) => row.id),
    ["c", "b"],
  );
  assert.equal(next.data[0], c);
  assert.deepEqual(next.range, {
    start: null,
    end: "9",
  });
});
test("gaps and invalid coverage leave the previous state and sequence intact", () => {
  const state = new WatchedOrders();
  state.apply(snapshot([row("a", "1")]));
  assert.throws(() => state.apply(change(2, [], [{ venue_id: "v", id: "a" }])));
  assert.throws(() =>
    state.apply(change(0, [row("outside", "z")], [{ venue_id: "v", id: "a" }])),
  );
  assert.deepEqual(
    state.apply(change(0)).data.map((row) => row.id),
    ["a"],
  );
  assert.deepEqual(state.apply(snapshot([], 2, true)).data, []);
  state.clear();
  assert.equal(state.apply(snapshot([row("fresh", "2")])).data[0].id, "fresh");
});

test("rank collision failures preserve both indexes for the next valid atomic batch", () => {
  const state = new WatchedOrders();
  state.apply(snapshot([row("a", "1"), row("b", "2")]));
  assert.throws(() =>
    state.apply(change(0, [row("c", "2")], [{ venue_id: "v", id: "a" }])),
  );
  assert.deepEqual(
    state.apply(change(0, [row("a", "3")])).data.map((row) => row.id),
    ["b", "a"],
  );
});
