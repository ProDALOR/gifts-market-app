import assert from "node:assert/strict";
import { test } from "node:test";
import {
  OrderActivityTracker,
  orderKey,
  FRESHNESS_MS,
} from "../src/orderbook/order-activity.ts";
import type { Order, Page } from "../src/market.ts";
const order = (id: string, venue = "venue") =>
  ({
    id,
    book: { venue_id: venue, collection_id: "gift", currency_id: "GRAM" },
  }) as Order;
const page = (ids: string[], next = "") =>
  ({
    data: ids.map((id) => order(id)),
    next_page_token: next,
    cursor: "view",
    sync: { ready: true, state: "live", updated_at: null },
  }) as Page<Order>;
function apply(
  tracker: OrderActivityTracker,
  pages: Page<Order>[],
  time: number,
  refreshed = new Set(pages.map((_, i) => i)),
) {
  return tracker.update(
    pages,
    pages.flatMap((page) => page.data),
    refreshed,
    time,
  );
}
test("initial loads and pagination are baselines; only new live head arrivals get ribbons", () => {
  const tracker = new OrderActivityTracker();
  assert.equal(
    apply(tracker, [page(["a", "b"], "tail")], 100).activity.size,
    0,
  );
  const paged = apply(tracker, [page(["a", "b"], "tail"), page(["c"])], 200);
  assert.equal(paged.activity.size, 0);
  const next = apply(
    tracker,
    [page(["x", "a"], "shifted"), page(["b", "c"])],
    300,
  );
  assert.deepEqual(next.activity.get(orderKey(order("x"))), {
    state: "new",
    since: 300,
  });
  assert.equal(next.activity.size, 1);
});
test("pushed tails stay unmarked; covered disappearances remain grey with venue-scoped identity", () => {
  const tracker = new OrderActivityTracker();
  apply(tracker, [page(["a", "b", "c"], "tail")], 0);
  const pushed = apply(tracker, [page(["x", "a", "b"], "tail")], 100);
  assert.ok(!pushed.activity.has(orderKey(order("c"))));
  const gone = apply(tracker, [page(["x", "b"], "tail")], 200);
  assert.deepEqual(
    gone.displayOrders.map((order) => order.id),
    ["x", "a", "b"],
  );
  assert.equal(gone.activity.get(orderKey(order("a")))?.state, "removed");
  assert.equal(gone.activity.get(orderKey(order("x")))?.since, 100);
  assert.deepEqual(tracker.liveRange(0, 3), { start: 0, end: 2 });
  assert.deepEqual(tracker.liveRange(1, 2), { start: 1, end: 1 });
  assert.deepEqual(tracker.liveRange(2, 3), { start: 1, end: 2 });
  const restored = apply(tracker, [page(["x", "a", "b"], "tail")], 300);
  assert.equal(restored.activity.get(orderKey(order("a")))?.state, "new");
  assert.equal(
    restored.displayOrders.filter((order) => order.id === "a").length,
    1,
  );
  assert.notEqual(orderKey(order("a")), orderKey(order("a", "another-venue")));
  assert.deepEqual(tracker.liveRange(1, 3), { start: 1, end: 3 });
});
test("complete empty heads retain disappearances for sixty seconds and cap their count", () => {
  const tracker = new OrderActivityTracker();
  const ids = Array.from({ length: 100 }, (_, i) => String(i));
  apply(tracker, [page(ids)], 100);
  const removed = apply(tracker, [page([])], 200);
  assert.equal(removed.displayOrders.length, 50);
  assert.equal(tracker.nextExpiry, 200 + FRESHNESS_MS);
  tracker.expire(200 + FRESHNESS_MS - 1);
  assert.equal(tracker.view([]).displayOrders.length, 50);
  tracker.expire(200 + FRESHNESS_MS);
  assert.equal(tracker.view([]).displayOrders.length, 0);
  assert.equal(tracker.view([]).activity.size, 0);
});
test("reconnects reset discovery and removed rows restored by pagination never stay grey", () => {
  const tracker = new OrderActivityTracker();
  apply(tracker, [page(["a", "b"])], 0);
  apply(tracker, [page(["b"])], 100);
  const restored = apply(tracker, [page(["b"], "tail"), page(["a"])], 200);
  assert.ok(!restored.activity.has(orderKey(order("a"))));
  tracker.rebase();
  const connected = apply(tracker, [page(["other"])], 300);
  assert.equal(connected.activity.size, 0);
  assert.deepEqual(
    connected.displayOrders.map((order) => order.id),
    ["other"],
  );
});
test("display offsets account for grey cards removed in different updates", () => {
  const tracker = new OrderActivityTracker();
  apply(tracker, [page(["a", "b", "c"])], 0);
  apply(tracker, [page(["b", "c"])], 100);
  const gone = apply(tracker, [page(["c"])], 200);
  assert.equal(gone.displayOrders.length, 3);
  assert.deepEqual(tracker.liveRange(0, 2), { start: 0, end: 0 });
  assert.deepEqual(tracker.liveRange(2, 3), { start: 0, end: 1 });
  tracker.expire(FRESHNESS_MS + 100);
  tracker.view([order("c")]);
  assert.deepEqual(tracker.liveRange(1, 2), { start: 0, end: 1 });
});
