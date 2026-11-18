import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { createOrderFeed, type FeedState } from "../src/order-feed.ts";
import { liveSocket } from "../src/live-socket.ts";
import type { Order, Page, Query } from "../src/market.ts";
const sync = { ready: true, state: "live" as const, updated_at: null };
const order = (id: string): Order => ({
  id,
  book: { venue_id: "v", collection_id: "gift", currency_id: "GRAM" },
  kind: "ask",
  quantity: 1,
  price: "1",
  observed_at: "2026-11-18T12:00:00Z",
  target: {
    kind: "criteria",
    criteria: { models: [], backdrops: [], patterns: [], number: null },
  },
});
const page = (ids: string[], next = ""): Page<Order> => ({
  data: ids.map(order),
  next_page_token: next,
  cursor: "r",
  sync,
});
const query: Query = {
  filters: { collection_id: "gift", currency_id: "GRAM" },
  side: "ask",
  sort: "price_asc",
  limit: 50,
  page_token: "",
};
const window = (ids: string[], before = false, after = true) => ({
  data: ids.map((id) => ({ ...order(id), position: `p.${id}` })),
  anchor: ids.length ? `p.${ids[Math.min(50, ids.length - 1)]}` : null,
  has_before: before,
  has_after: after,
  cursor: "r",
});
function live(t: TestContext) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  t.mock.method(globalThis, "fetch", () =>
    assert.fail("Live windows must not load HTTP pages"),
  );
  let receive: (frame: any) => void = () => {},
    status: (state: string) => void = () => {};
  const commands: any[] = [];
  let releases = 0;
  t.mock.method(liveSocket, "subscribe", (command, frame, state) => {
    commands.push(command);
    receive = frame;
    status = state ?? (() => {});
    return () => releases++;
  });
  t.mock.method(liveSocket, "update", (command) => {
    commands.push(command);
  });
  t.mock.method(liveSocket, "remember", () => {});
  let state: FeedState | undefined;
  const feed = createOrderFeed(
    { kind: "orders", query },
    false,
    (next) => (state = next),
  );
  t.after(() => feed.close());
  feed.start();
  return {
    feed,
    commands,
    get state() {
      return state!;
    },
    get releases() {
      return releases;
    },
    deliver(ids: string[], before = false, after = true) {
      receive({
        type: "window",
        id: "offers",
        window: window(ids, before, after),
        sync,
        revision: "r",
      });
    },
    frame(frame: any) {
      receive(frame);
    },
    status,
  };
}
test("live updates replace one complete window atomically without appending old pages", (t) => {
  const f = live(t);
  assert.equal(f.commands[0].type, "subscribe_window");
  assert.equal(f.commands[0].query.anchor, null);
  f.deliver(["a", "b", "c"]);
  assert.deepEqual(
    f.state.orders.map((o) => o.id),
    ["a", "b", "c"],
  );
  f.deliver(["x", "a", "b"]);
  assert.deepEqual(
    f.state.orders.map((o) => o.id),
    ["x", "a", "b"],
  );
  assert.equal(f.state.activity.get(JSON.stringify(["v", "x"]))?.state, "new");
  assert.ok(
    !f.state.activity.has(JSON.stringify(["v", "c"])),
    "Leaving the window tail is not disappearance",
  );
  f.deliver(["x", "b"], false, false);
  assert.equal(
    f.state.activity.get(JSON.stringify(["v", "a"]))?.state,
    "removed",
  );
});
test("scrolling requests anchored windows in both directions and retains the old window until replacement", (t) => {
  const f = live(t),
    ids = Array.from({ length: 150 }, (_, i) => String(i));
  f.deliver(ids);
  f.feed.setVisibleRange(110, 140, 120);
  t.mock.timers.tick(80);
  assert.equal(f.commands.length, 2);
  assert.equal(f.commands[1].query.anchor, "p.120");
  assert.equal(f.state.loadingMore, true);
  assert.equal(f.state.orders[0].id, "0");
  f.deliver(
    Array.from({ length: 150 }, (_, i) => String(i + 70)),
    true,
  );
  assert.equal(f.state.loadingMore, false);
  assert.equal(f.state.orders.length, 150);
  assert.equal(f.state.orders[0].id, "70");
  assert.equal(f.state.activity.size, 0);
  f.feed.setVisibleRange(5, 30, 10);
  t.mock.timers.tick(80);
  assert.equal(f.commands[2].query.anchor, "p.80");
  f.deliver(
    Array.from({ length: 150 }, (_, i) => String(i + 30)),
    true,
  );
  assert.equal(f.state.orders[0].id, "30");
  assert.equal(f.state.orders.length, 150);
});
test("scroll intent is debounced, active requests are bounded and cancellation releases the lease", (t) => {
  const f = live(t);
  f.deliver(
    Array.from({ length: 150 }, (_, i) => String(i)),
    true,
  );
  f.feed.setVisibleRange(1, 10, 2);
  t.mock.timers.tick(40);
  f.feed.setVisibleRange(6, 20, 7);
  t.mock.timers.tick(79);
  assert.equal(f.commands.length, 1);
  t.mock.timers.tick(1);
  assert.equal(f.commands[1].query.anchor, "p.7");
  f.feed.setVisibleRange(130, 149, 140);
  t.mock.timers.tick(1000);
  assert.equal(f.commands.length, 2);
  f.feed.close();
  assert.equal(f.releases, 1);
});
test("freshness frames preserve data and reconnect establishes a new animation baseline", (t) => {
  const f = live(t);
  f.deliver(["a", "b"]);
  const data = f.state.orders;
  f.frame({ type: "sync", sync: { ...sync, state: "stale" }, revision: "new" });
  assert.equal(f.state.orders, data);
  assert.equal(f.state.status, "Stale");
  f.status("Reconnecting");
  f.deliver(["x", "a", "b"]);
  assert.equal(f.state.activity.size, 0);
});
test("anchored window edge changes are not reported as sales or fresh arrivals", (t) => {
  const f = live(t);
  f.deliver(["a", "b", "c"], true);
  f.deliver(["b", "c", "d"], true);
  assert.equal(f.state.activity.size, 0);
  assert.deepEqual(
    f.state.displayOrders.map((o) => o.id),
    ["b", "c", "d"],
  );
});
test("scrolling back during an in-flight move keeps the displayed window until the latest anchor arrives", (t) => {
  const f = live(t);
  f.deliver(Array.from({ length: 150 }, (_, i) => String(i)));
  f.feed.setVisibleRange(110, 140, 120);
  t.mock.timers.tick(80);
  f.feed.setVisibleRange(0, 30, 10);
  f.deliver(
    Array.from({ length: 150 }, (_, i) => String(i + 70)),
    true,
  );
  assert.equal(f.state.orders[0].id, "0");
  assert.equal(f.state.loadingMore, true);
  assert.equal(f.commands.at(-1).query.anchor, "p.10");
  f.deliver(
    Array.from({ length: 150 }, (_, i) => String(i)),
    false,
  );
  assert.equal(f.state.loadingMore, false);
  assert.equal(f.state.orders[10].id, "10");
});
test("interior removals are visible in anchored windows while edge shifts remain unclassified", (t) => {
  const f = live(t);
  f.deliver(["a", "b", "c", "d"], true, true);
  f.deliver(["a", "c", "d", "e"], true, true);
  assert.equal(
    f.state.activity.get(JSON.stringify(["v", "b"]))?.state,
    "removed",
  );
  assert.equal(f.state.activity.has(JSON.stringify(["v", "e"])), false);
});

test("pausing deep in a window freezes around its anchor and releases the snapshot on close", async (t) => {
  const calls: any[] = [];
  t.mock.method(globalThis, "fetch", async (url: string, init: RequestInit) => {
    const body = JSON.parse(init.body as string);
    calls.push({ method: init.method, body });
    if (init.method === "DELETE") return new Response(null, { status: 204 });
    assert.equal(url, "/orders/window");
    assert.equal(body.query.anchor, "p.2000");
    return Response.json({
      window: window(
        Array.from({ length: 150 }, (_, i) => String(1950 + i)),
        true,
        true,
      ),
      snapshot: "frozen.1",
      sync,
      revision: "r1",
    });
  });
  let state: FeedState | undefined;
  const feed = createOrderFeed(
    { kind: "orders", query },
    true,
    (next) => (state = next),
    { kind: "window", anchor: "p.2000" },
  );
  feed.start();
  await new Promise(setImmediate);
  assert.equal(state!.orders[50].id, "2000");
  assert.equal(state!.status, "Paused");
  feed.setVisibleRange(50, 80, 50);
  assert.deepEqual(feed.bookmark, { kind: "window", anchor: "p.2000" });
  feed.close();
  await new Promise(setImmediate);
  assert.deepEqual(calls.at(-1), {
    method: "DELETE",
    body: { snapshot: "frozen.1" },
  });
});
test("frozen navigation reuses its lease and expiration restarts around the current anchor", async (t) => {
  const bodies: any[] = [];
  let fail = false;
  t.mock.method(globalThis, "fetch", async (_: string, init: RequestInit) => {
    if (init.method === "DELETE") return new Response(null, { status: 204 });
    const body = JSON.parse(init.body as string);
    bodies.push(body);
    if (body.snapshot && fail) {
      fail = false;
      return Response.json(
        { error: { code: "restart_read", message: "Expired" } },
        { status: 409 },
      );
    }
    const pivot = body.query.anchor ? Number(body.query.anchor.slice(2)) : 0;
    return Response.json({
      window: window(
        Array.from({ length: 150 }, (_, i) =>
          String(Math.max(0, pivot - 50) + i),
        ),
        pivot > 50,
        true,
      ),
      snapshot: body.snapshot ?? `lease.${bodies.length}`,
      sync,
      revision: "r",
    });
  });
  let state: FeedState | undefined;
  const feed = createOrderFeed(
    { kind: "orders", query },
    true,
    (next) => (state = next),
  );
  feed.start();
  await new Promise(setImmediate);
  feed.setVisibleRange(110, 140, 120);
  feed.loadMore();
  await new Promise(setImmediate);
  assert.equal(bodies[1].snapshot, "lease.1");
  assert.equal(bodies[1].query.anchor, "p.120");
  fail = true;
  feed.setVisibleRange(110, 140, 120);
  feed.loadMore();
  await new Promise(setImmediate);
  await new Promise(setImmediate);
  assert.equal(bodies.at(-1).snapshot, null);
  assert.equal(bodies.at(-1).query.anchor, "p.190");
  assert.equal(state!.status, "Paused");
  feed.close();
});
test("late frozen responses release their lease without updating a closed view", async (t) => {
  let finish: (response: Response) => void = () => {};
  const released: string[] = [];
  t.mock.method(globalThis, "fetch", async (_: string, init: RequestInit) => {
    if (init.method === "DELETE") {
      released.push(JSON.parse(init.body as string).snapshot);
      return new Response(null, { status: 204 });
    }
    return new Promise<Response>((resolve) => (finish = resolve));
  });
  let state: FeedState | undefined;
  const feed = createOrderFeed(
    { kind: "orders", query },
    true,
    (next) => (state = next),
  );
  feed.start();
  feed.close();
  const closed = state;
  finish(
    Response.json({
      window: window(["late"]),
      snapshot: "orphan",
      sync,
      revision: "r",
    }),
  );
  await new Promise(setImmediate);
  assert.equal(state, closed);
  assert.deepEqual(released, ["orphan"]);
});
test("history alone uses forward snapshot pagination and restarts expired traversals", async (t) => {
  let expired = true;
  const tokens: string[] = [];
  t.mock.method(globalThis, "fetch", async (url: string, init: RequestInit) => {
    assert.equal(url, "/history/query");
    const body = JSON.parse(init.body as string);
    assert.equal(body.consistency, undefined);
    const token = body.page_token;
    tokens.push(token);
    if (token && expired) {
      expired = false;
      return Response.json(
        { error: { code: "restart_read", message: "Expired" } },
        { status: 409 },
      );
    }
    return Response.json(page(token ? ["c"] : ["a", "b"], token ? "" : "next"));
  });
  let state: FeedState | undefined;
  const feed = createOrderFeed(
    { kind: "history", query: { filters: query.filters, limit: 2 } },
    false,
    (next) => (state = next),
    { kind: "history", depth: 2 },
  );
  feed.start();
  await new Promise(setImmediate);
  assert.deepEqual(
    state!.orders.map((o) => o.id),
    ["a", "b", "c"],
  );
  assert.deepEqual(tokens, ["", "next", "", "next"]);
  feed.close();
});
