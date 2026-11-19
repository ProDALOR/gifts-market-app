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
const apply = (tracker: OrderActivityTracker, ids: string[], time: number) =>
  tracker.update(
    ids.map((id) => order(id)),
    time,
  );
test("range arrival and disappearance are explicit, including boundary rows", () => {
  const tracker = new OrderActivityTracker();
  assert.equal(apply(tracker, ["a", "b", "c"], 0).activity.size, 0);
  const pushed = apply(tracker, ["x", "a", "b"], 100);
  assert.equal(pushed.activity.get(orderKey(order("x")))?.state, "new");
  assert.equal(pushed.activity.get(orderKey(order("c")))?.state, "removed");
  const gone = apply(tracker, ["x", "b"], 200);
  assert.deepEqual(
    gone.displayOrders.map((o) => o.id),
    ["x", "a", "c", "b"],
  );
  assert.equal(gone.activity.get(orderKey(order("a")))?.state, "removed");
  assert.deepEqual(tracker.liveRange(0, 4), { start: 0, end: 2 });
  const restored = apply(tracker, ["x", "a", "b"], 300);
  assert.equal(restored.activity.get(orderKey(order("a")))?.state, "new");
});
test("range boundaries do not suppress confirmed arrivals or removals", () => {
  const tracker = new OrderActivityTracker();
  apply(tracker, ["a", "b", "c", "d"], 0);
  const next = apply(tracker, ["b", "x", "d", "e"], 100);
  assert.equal(next.activity.get(orderKey(order("a")))?.state, "removed");
  assert.equal(next.activity.get(orderKey(order("e")))?.state, "new");
  assert.equal(next.activity.get(orderKey(order("x")))?.state, "new");
  assert.equal(next.activity.get(orderKey(order("c")))?.state, "removed");
});
test("rebase resets animations and expiry releases retained grey rows", () => {
  const tracker = new OrderActivityTracker();
  apply(tracker, ["a", "b"], 0);
  apply(tracker, ["b"], 100);
  tracker.expire(FRESHNESS_MS + 100);
  assert.equal(tracker.view([order("b")]).displayOrders.length, 1);
  tracker.rebase();
  assert.equal(apply(tracker, ["other"], FRESHNESS_MS + 200).activity.size, 0);
});
test("identities remain venue scoped and retained rows do not affect live offsets", () => {
  const tracker = new OrderActivityTracker();
  tracker.update([order("a"), order("a", "other"), order("b")], 0);
  const next = tracker.update([order("a", "other"), order("b")], 100);
  assert.equal(next.activity.get(orderKey(order("a")))?.state, "removed");
  assert.ok(!next.activity.has(orderKey(order("a", "other"))));
  assert.deepEqual(tracker.liveRange(0, 3), { start: 0, end: 2 });
});
