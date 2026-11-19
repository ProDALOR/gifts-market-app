import assert from "node:assert/strict";
import { test, beforeEach, type TestContext } from "node:test";
import { createOrderFeed, type FeedState } from "../src/order-feed.ts";
import { liveSocket } from "../src/live-socket.ts";
import type { Order, OrderQueryResult, FeedQuery } from "../src/market.ts";
let statusCallback: (sync: any) => void = () => {};
beforeEach((t) => {
  t.mock.method(liveSocket, "watchStatus", (callback) => {
    statusCallback = callback;
    callback("available");
    return () => {};
  });
});
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
const page = (ids: string[], next = ""): OrderQueryResult => ({
  data: ids.map((id) => ({ ...order(id), key: id })),
  next: next || null,
  range: { start: null, end: null },
  snapshot: "lease",
});
const query: Extract<FeedQuery, { kind: "orders" }>["query"] = {
  filters: { collection_id: "gift", currency_id: "GRAM" },
  side: "ask",
  sort: "price_asc",
};
const result = (ids: string[], before = false, after = true) => {
  const data = ids.map((id) => ({
    ...order(id),
    key: `${String(/^\d+$/.test(id) ? Number(id) + 1 : id === "x" ? 0 : 1).padStart(16, "0")}:${id}`,
  }));
  const boundary = (row: (typeof data)[number]) => row.key;
  return {
    data,
    next: null,
    range: {
      start: before && data.length ? boundary(data[0]) : null,
      end: after && data.length ? boundary(data.at(-1)!) : null,
    },
  };
};
function live(t: TestContext) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  t.mock.method(globalThis, "fetch", () =>
    assert.fail("Live windows must not load HTTP pages"),
  );
  let receive: (frame: any) => void = () => {},
    status: (state: string) => void = () => {};
  const commands: any[] = [];
  let previous: any;
  let sequence = -1;
  let releases = 0;
  t.mock.method(liveSocket, "subscribe", (command, frame, state) => {
    commands.push(command);
    previous = undefined;
    sequence = -1;
    receive = frame;
    status = state ?? (() => {});
    return () => releases++;
  });
  t.mock.method(liveSocket, "update", (command) => {
    commands.push(command);
    previous = undefined;
    sequence = -1;
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
      const next = result(ids, before, after);
      const old = previous;
      previous = next;
      sequence++;
      if (!old)
        receive({
          type: "orders.snapshot",
          id: "offers",
          ...next,
          sequence,
          reset: false,
        });
      else {
        const oldRows = new Map(old.data.map((row: any) => [row.id, row]));
        receive({
          type: "orders.changed",
          id: "offers",
          base_sequence: sequence - 1,
          sequence,
          upserts: next.data.filter(
            (row) =>
              JSON.stringify(oldRows.get(row.id)) !== JSON.stringify(row),
          ),
          removed: old.data
            .filter((row: any) => !ids.includes(row.id))
            .map((row: any) => ({ venue_id: row.book.venue_id, id: row.id })),
        });
      }
    },
    frame(frame: any) {
      if (frame.type === "error") {
        previous = undefined;
        sequence = -1;
      }
      if (frame.type === "market.status") statusCallback(frame.status);
      else receive(frame);
    },
    status(value: string) {
      previous = undefined;
      sequence = -1;
      status(value);
    },
  };
}
test("live windows refresh their covered region while keeping previously loaded edges", (t) => {
  const f = live(t);
  assert.equal(f.commands[0].type, "orders.watch");
  assert.deepEqual(f.commands[0].params.query.navigation, { type: "head" });
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
  assert.equal(
    f.state.activity.get(JSON.stringify(["v", "c"]))?.state,
    "removed",
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
  assert.deepEqual(f.commands[1].params.query.navigation, {
    type: "around",
    key: result(["120"]).data[0].key,
  });
  assert.equal(f.state.loadingMore, true);
  assert.equal(f.state.orders[0].id, "0");
  f.deliver(
    Array.from({ length: 150 }, (_, i) => String(i + 70)),
    true,
  );
  assert.equal(f.state.loadingMore, false);
  assert.equal(f.state.orders.length, 220);
  assert.equal(f.state.orders[0].id, "0");
  assert.equal(f.state.activity.size, 0);
  f.feed.setVisibleRange(5, 30, 10);
  t.mock.timers.tick(80);
  assert.deepEqual(f.commands[2].params.query.navigation, {
    type: "around",
    key: result(["10"]).data[0].key,
  });
  f.deliver(
    Array.from({ length: 150 }, (_, i) => String(i)),
    false,
  );
  assert.equal(f.state.orders[0].id, "0");
  assert.equal(f.state.orders.length, 220);
  assert.equal(f.state.orders.at(-1)?.id, "219");
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
  assert.deepEqual(f.commands[1].params.query.navigation, {
    type: "around",
    key: result(["7"]).data[0].key,
  });
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
  f.frame({ type: "market.status", status: "delayed" });
  assert.equal(f.state.orders, data);
  assert.equal(f.state.status, "Updates delayed");
  f.status("Reconnecting");
  f.deliver(["x", "a", "b"]);
  assert.equal(f.state.activity.size, 0);
});
test("anchored window edge changes are not reported as sales or fresh arrivals", (t) => {
  const f = live(t);
  f.deliver(["a", "b", "c"], true);
  f.deliver(["b", "c"], true);
  assert.equal(
    f.state.activity.get(JSON.stringify(["v", "a"]))?.state,
    "removed",
  );
  assert.deepEqual(
    f.state.displayOrders.map((o) => o.id),
    ["a", "b", "c"],
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
  assert.deepEqual(f.commands.at(-1).params.query.navigation, {
    type: "around",
    key: result(["10"]).data[0].key,
  });
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
  f.deliver(["a", "c", "d"], true, true);
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
    assert.equal(url, "/orders/query");
    assert.equal(body.navigation?.key, result(["2000"]).data[0].key);
    assert.equal(body.view.kind, "snapshot");
    return Response.json({
      ...result(
        Array.from({ length: 150 }, (_, i) => String(1950 + i)),
        true,
        true,
      ),
      snapshot: "frozen.1",
    });
  });
  let state: FeedState | undefined;
  const feed = createOrderFeed(
    { kind: "orders", query },
    true,
    (next) => (state = next),
    { kind: "range", anchor: result(["2000"]).data[0].key },
  );
  feed.start();
  await new Promise(setImmediate);
  assert.equal(state!.orders[50].id, "2000");
  assert.equal(state!.status, "Paused");
  feed.setVisibleRange(50, 80, 50);
  assert.equal(feed.bookmark.kind, "range");
  if (feed.bookmark.kind === "range") {
    assert.equal(feed.bookmark.anchor, result(["2000"]).data[0].key);
    assert.equal(feed.bookmark.cache?.rows.length, 150);
  }
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
    if (body.view.id && fail) {
      fail = false;
      return Response.json(
        { error: { code: "restart_read", message: "Expired" } },
        { status: 409 },
      );
    }
    const pivot = body.navigation?.key
      ? Number(body.navigation?.key.split(":")[1])
      : 0;
    return Response.json({
      ...result(
        Array.from({ length: 150 }, (_, i) =>
          String(Math.max(0, pivot - 50) + i),
        ),
        pivot > 50,
        true,
      ),
      snapshot: body.view.id ?? `lease.${bodies.length}`,
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
  assert.equal(bodies[1].view.id, "lease.1");
  assert.equal(bodies[1].navigation.key, result(["120"]).data[0].key);
  fail = true;
  feed.setVisibleRange(170, 210, 190);
  feed.loadMore();
  await new Promise(setImmediate);
  await new Promise(setImmediate);
  assert.equal(bodies.at(-1).view.id, null);
  assert.equal(bodies.at(-1).navigation.key, result(["190"]).data[0].key);
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
      ...result(["late"]),
      snapshot: "orphan",
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
    assert.equal(url, "/orders/query");
    const body = JSON.parse(init.body as string);
    if (init.method === "DELETE") return new Response(null, { status: 204 });
    assert.equal(body.view.kind, "snapshot");
    const token = body.navigation?.key ?? "";
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

test("pausing and resuming keep every visited region while refreshing the visible window", async (t) => {
  const f = live(t);
  f.deliver(Array.from({ length: 150 }, (_, i) => String(i)));
  f.feed.setVisibleRange(110, 140, 120);
  t.mock.timers.tick(80);
  f.deliver(
    Array.from({ length: 150 }, (_, i) => String(i + 70)),
    true,
  );
  f.feed.setVisibleRange(140, 170, 150);
  const bookmark = f.feed.bookmark;
  f.feed.close();
  t.mock.method(globalThis, "fetch", async (_: string, init: RequestInit) => {
    if (init.method === "DELETE") return new Response(null, { status: 204 });
    const body = JSON.parse(init.body as string);
    assert.equal(body.navigation?.key, result(["150"]).data[0].key);
    return Response.json({
      ...result(
        Array.from({ length: 150 }, (_, i) => String(i + 100)),
        true,
      ),
      snapshot: "paused",
    });
  });
  let frozen: FeedState | undefined;
  const feed = createOrderFeed(
    { kind: "orders", query },
    true,
    (next) => (frozen = next),
    bookmark,
  );
  t.after(() => feed.close());
  feed.start();
  await new Promise(setImmediate);
  assert.equal(frozen!.orders.length, 250);
  assert.equal(frozen!.orders[0].id, "0");
  assert.equal(frozen!.orders.at(-1)?.id, "249");
  assert.equal(frozen!.status, "Paused");
  const resumedBookmark = feed.bookmark;
  feed.close();
  let resumed: FeedState | undefined;
  const liveAgain = createOrderFeed(
    { kind: "orders", query },
    false,
    (next) => (resumed = next),
    resumedBookmark,
  );
  t.after(() => liveAgain.close());
  liveAgain.start();
  f.deliver(
    Array.from({ length: 150 }, (_, i) => String(i + 100)),
    true,
  );
  assert.equal(resumed!.orders.length, 250);
  assert.equal(resumed!.orders[0].id, "0");
  assert.equal(resumed!.orders.at(-1)?.id, "249");
  assert.equal(resumed!.status, "Live");
});

test("sequence gaps retain visited rows and retry only the affected watch with backoff", (t) => {
  const f = live(t);
  let retries = 0;
  t.mock.method(liveSocket, "retry", (id) => {
    assert.equal(id, "offers");
    retries++;
  });
  f.deliver(["a", "b"]);
  f.frame({
    type: "orders.changed",
    base_sequence: 99,
    sequence: 100,
    upserts: [],
    removed: [],

    revision: "bad",
  });
  assert.deepEqual(
    f.state.orders.map((row) => row.id),
    ["a", "b"],
  );
  assert.equal(f.state.status, "Reconnecting");
  assert.equal(retries, 0);
  t.mock.timers.tick(2000);
  assert.equal(retries, 1);
  f.status("Connecting");
  f.deliver(["a", "b"]);
  assert.equal(f.state.status, "Live");
});
test("closing a watch cancels pending sequence recovery", (t) => {
  const f = live(t);
  let retries = 0;
  t.mock.method(liveSocket, "retry", () => retries++);
  f.deliver(["a"]);
  f.frame({
    type: "orders.changed",
    base_sequence: 99,
    sequence: 100,
    upserts: [],
    removed: [],

    revision: "bad",
  });
  f.feed.close();
  t.mock.timers.tick(2000);
  assert.equal(retries, 0);
});

test("revisiting cached rows moves away from an active range that became empty", (t) => {
  const f = live(t);
  f.deliver(Array.from({ length: 150 }, (_, i) => String(i)));
  f.feed.setVisibleRange(110, 140, 120);
  t.mock.timers.tick(80);
  f.deliver(
    Array.from({ length: 150 }, (_, i) => String(i + 100)),
    true,
  );
  f.deliver([], true);
  assert.equal(f.state.orders.length, 100);
  f.feed.setVisibleRange(0, 20, 10);
  t.mock.timers.tick(80);
  assert.deepEqual(f.commands.at(-1).params.query.navigation, {
    type: "around",
    key: result(["10"]).data[0].key,
  });
  assert.equal(f.state.loadingMore, true);
  f.deliver(Array.from({ length: 150 }, (_, i) => String(i)));
  assert.equal(f.state.loadingMore, false);
});

test("an emptied range queries beyond its boundary once instead of leaving the GUI stranded", (t) => {
  const f = live(t);
  f.deliver(Array.from({ length: 150 }, (_, i) => String(i)));
  f.deliver([]);
  assert.equal(f.state.orders.length, 0);
  t.mock.timers.tick(80);
  assert.deepEqual(f.commands.at(-1).params.query.navigation, {
    type: "after",
    key: result(["149"]).data[0].key,
  });
  const count = f.commands.length;
  f.deliver([], false, false);
  t.mock.timers.tick(1000);
  assert.equal(
    f.commands.length,
    count,
    "a globally empty query must not poll or loop",
  );
});
