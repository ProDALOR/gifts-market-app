import assert from "node:assert/strict";
import { test } from "node:test";
import { WatchedOrders } from "../src/feed/watched-orders.ts";
const row = (id: string, key: string) =>
  ({
    id,
    key: key,
    book: { venue_id: "v", collection_id: "g", currency_id: "GRAM" },
  }) as any;
const snapshot = (data: any[], sequence = 0) =>
  ({
    type: "orders.snapshot",
    next: null,
    sequence,
    data,
    range: { start: null, end: "9" },
  }) as any;
const change = (base: number, upserts: any[] = [], removed: any[] = []) =>
  ({
    type: "orders.changed",
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
  assert.deepEqual(state.apply(snapshot([], 2)).data, []);
  state.clear();
  assert.equal(state.apply(snapshot([row("fresh", "2")])).data[0].id, "fresh");
});

test("rank collision failures preserve the snapshot for the next valid atomic batch", () => {
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

test("simultaneous repricing merges atomically and preserves unaffected rows", () => {
  const state = new WatchedOrders();
  const untouched = row("d", "44");
  state.apply(
    snapshot([row("a", "11"), row("b", "22"), row("c", "33"), untouched]),
  );
  const swapped = state.apply(change(0, [row("b", "12"), row("a", "21")]));
  assert.deepEqual(
    swapped.data.map((row) => row.id),
    ["b", "a", "c", "d"],
  );
  assert.equal(swapped.data[3], untouched);
  const cycled = state.apply(
    change(1, [row("c", "13"), row("b", "22"), row("a", "31")]),
  );
  assert.deepEqual(
    cycled.data.map((row) => row.id),
    ["c", "b", "a", "d"],
  );
  assert.equal(cycled.data[3], untouched);
});

test("one batch can remove an order and move another order into its former rank", () => {
  const state = new WatchedOrders();
  state.apply(snapshot([row("a", "11"), row("b", "22"), row("c", "33")]));
  const result = state.apply(
    change(0, [row("b", "12")], [{ venue_id: "v", id: "a" }]),
  );
  assert.deepEqual(
    result.data.map((row) => row.id),
    ["b", "c"],
  );
});
