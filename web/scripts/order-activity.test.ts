import assert from "node:assert/strict";
import { test } from "node:test";
import {
  OrderActivityTracker,
  orderKey,
  FRESHNESS_MS,
} from "../src/orderbook/order-activity.ts";
import type { Order } from "../src/market.ts";
const order = (id: string, venue = "venue") =>
  ({
    id,
    book: { venue_id: venue, collection_id: "gift", currency_id: "GRAM" },
  }) as Order;
const apply = (
  tracker: OrderActivityTracker,
  ids: string[],
  time: number,
  before = false,
  after = true,
) =>
  tracker.update(
    ids.map((id) => order(id)),
    { before, after },
    time,
  );
test("window arrival and disappearance apply only within confirmed coverage", () => {
  const tracker = new OrderActivityTracker();
  assert.equal(apply(tracker, ["a", "b", "c"], 0).activity.size, 0);
  const pushed = apply(tracker, ["x", "a", "b"], 100);
  assert.equal(pushed.activity.get(orderKey(order("x")))?.state, "new");
  assert.ok(!pushed.activity.has(orderKey(order("c"))));
  const gone = apply(tracker, ["x", "b"], 200);
  assert.deepEqual(
    gone.displayOrders.map((o) => o.id),
    ["x", "a", "b"],
  );
  assert.equal(gone.activity.get(orderKey(order("a")))?.state, "removed");
  assert.deepEqual(tracker.liveRange(0, 3), { start: 0, end: 2 });
  const restored = apply(tracker, ["x", "a", "b"], 300);
  assert.equal(restored.activity.get(orderKey(order("a")))?.state, "new");
});
test("both edges of anchored windows stay unclassified, interior changes are tracked", () => {
  const tracker = new OrderActivityTracker();
  apply(tracker, ["a", "b", "c", "d"], 0, true, true);
  const next = apply(tracker, ["b", "x", "d", "e"], 100, true, true);
  assert.ok(!next.activity.has(orderKey(order("a"))));
  assert.ok(!next.activity.has(orderKey(order("e"))));
  assert.equal(next.activity.get(orderKey(order("x")))?.state, "new");
  assert.equal(next.activity.get(orderKey(order("c")))?.state, "removed");
});
test("rebase resets animations and expiry releases retained grey rows", () => {
  const tracker = new OrderActivityTracker();
  apply(tracker, ["a", "b"], 0, false, false);
  apply(tracker, ["b"], 100, false, false);
  tracker.expire(FRESHNESS_MS + 100);
  assert.equal(tracker.view([order("b")]).displayOrders.length, 1);
  tracker.rebase();
  assert.equal(apply(tracker, ["other"], FRESHNESS_MS + 200).activity.size, 0);
});
test("identities remain venue scoped and retained rows do not affect live offsets", () => {
  const tracker = new OrderActivityTracker();
  tracker.update(
    [order("a"), order("a", "other"), order("b")],
    { before: false, after: false },
    0,
  );
  const next = tracker.update(
    [order("a", "other"), order("b")],
    { before: false, after: false },
    100,
  );
  assert.equal(next.activity.get(orderKey(order("a")))?.state, "removed");
  assert.ok(!next.activity.has(orderKey(order("a", "other"))));
  assert.deepEqual(tracker.liveRange(0, 3), { start: 0, end: 2 });
});
