import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { createOrderFeed, type FeedState } from "../src/order-feed.ts";
import { liveSocket } from "../src/live-socket.ts";
import { LiveRanges } from "../src/feed/range-source.ts";
import { ApiFailure } from "../src/market.ts";
import type { OrderQuery, OrderQueryResult } from "../src/market.ts";
const tick = () => new Promise<void>((r) => setImmediate(r));
async function until(check: () => boolean) {
  for (let i = 0; i < 100; i++) {
    if (check()) return;
    await tick();
  }
  assert.fail("Feed did not settle");
}
const key = (n: number) => `K${String(n).padStart(7, "0")}`;
const row = (n: number) => ({
  id: String(n),
  key: key(1000 + n * 2),
  book: { venue_id: "v", collection_id: "gift", currency_id: "GRAM" },
  kind: "ask" as const,
  price: String(n + 1),
  quantity: 1,
  observed_at: "2026-11-18T12:00:00Z",
  target: {
    kind: "criteria" as const,
    models: [],
    backdrops: [],
    patterns: [],
  },
});
const query = {
  filters: { collection_id: "gift", currency_id: "GRAM" },
  side: "ask" as const,
  sort: "price_asc" as const,
};
function page(q: OrderQuery, rows: ReturnType<typeof row>[]): OrderQueryResult {
  const range = q.range ?? { start: null, end: null };
  const all = rows.filter(
    (r) =>
      (range.start == null || r.key >= range.start) &&
      (range.end == null || r.key <= range.end),
  );
  const data = all.slice(0, q.limit ?? 100),
    more = all.length > data.length;
  return {
    data,
    range: { start: range.start, end: more ? data.at(-1)!.key : range.end },
    next: more ? key(Number(data.at(-1)!.key.slice(1)) + 1) : null,
  };
}
function fixture(t: TestContext, frozen = false) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let rows = Array.from({ length: 250 }, (_, i) => row(i)),
    state: FeedState;
  const queries: OrderQuery[] = [],
    commands: any[] = [],
    signals: AbortSignal[] = [],
    releases: string[] = [];
  let active: any,
    receive: (frame: any) => void = () => {},
    previous: any,
    sequence = 0,
    status: (value: string) => void = () => {};
  let failure: Error | undefined, held: Promise<void> | undefined;
  let emptySeed = false;
  const snapshot = () => {
    if (!active) return;
    const seed = previous ? undefined : page(active.params, rows);
    if (seed && emptySeed) {
      emptySeed = false;
      seed.data = [];
    }
    const range = previous?.range ?? seed!.range;
    const data =
      seed?.data ??
      rows.filter(
        (r) =>
          (range.start == null || r.key >= range.start) &&
          (range.end == null || r.key <= range.end),
      );
    if (data.length > 100) {
      receive({
        type: "error",
        id: "offers",
        error: { code: "capacity_exceeded", message: "Narrow the range" },
      });
      return;
    }
    const next = { data, range, next: previous?.next ?? seed?.next ?? null };
    if (!previous)
      receive({ type: "orders.snapshot", id: "offers", sequence: 0, ...next });
    else
      receive({
        type: "orders.changed",
        id: "offers",
        sequence: ++sequence,
        upserts: data.filter(
          (r) =>
            JSON.stringify(
              previous.data.find((old: any) => old.id === r.id),
            ) !== JSON.stringify(r),
        ),
        removed: previous.data
          .filter((old: any) => !data.some((r) => r.id === old.id))
          .map((r: any) => ({ venue_id: "v", id: r.id })),
      });
    previous = next;
  };
  const read = async (q: OrderQuery, signal: AbortSignal) => {
    queries.push(q);
    signals.push(signal);
    await held;
    if (failure) {
      const e = failure;
      failure = undefined;
      throw e;
    }
    return page(q, rows);
  };
  t.mock.method(liveSocket, "watchStatus", (callback) => {
    callback("available");
    return () => {};
  });
  t.mock.method(liveSocket, "request", (call: any, signal: AbortSignal) =>
    read(call.params, signal),
  );
  t.mock.method(
    liveSocket,
    "subscribe",
    (command: any, callback: any, changed: any) => {
      active = command;
      commands.push(command);
      receive = callback;
      status = changed;
      previous = undefined;
      sequence = 0;
      queueMicrotask(snapshot);
      return () => {
        active = undefined;
      };
    },
  );
  t.mock.method(liveSocket, "update", (command: any) => {
    if (JSON.stringify(active) === JSON.stringify(command)) return false;
    active = command;
    commands.push(command);
    previous = undefined;
    sequence = 0;
    queueMicrotask(snapshot);
    return true;
  });
  t.mock.method(liveSocket, "retry", () => {
    previous = undefined;
    sequence = 0;
    queueMicrotask(snapshot);
  });
  t.mock.method(globalThis, "fetch", async (_path: any, init: RequestInit) => {
    if (!frozen) assert.fail("Live orders use WebSocket queries");
    if (init.method === "DELETE") {
      releases.push(JSON.parse(String(init.body)).snapshot);
      return new Response(null, { status: 204 });
    }
    const q = JSON.parse(String(init.body));
    const result = await read(q, init.signal as AbortSignal);
    return new Response(JSON.stringify({ ...result, snapshot: "lease" }));
  });
  const feed = createOrderFeed(
    { kind: "orders", query },
    frozen,
    (next) => (state = next),
  );
  t.after(() => feed.close());
  return {
    feed,
    queries,
    commands,
    signals,
    releases,
    start: async () => {
      feed.start();
      await until(() => feed.ready);
      await tick();
    },
    get state() {
      return state!;
    },
    get rows() {
      return rows;
    },
    setRows: (value: typeof rows) => {
      rows = value;
    },
    deliver: snapshot,
    skipSeed: () => {
      emptySeed = true;
    },
    reconnect: () => {
      previous = undefined;
      sequence = 0;
      status("Reconnecting");
      queueMicrotask(snapshot);
    },
    fail: (error: Error) => {
      failure = error;
    },
    hold: (value?: Promise<void>) => {
      held = value;
    },
  };
}
test("live initialization sends one watch without a preliminary query", async (t) => {
  const f = fixture(t);
  await f.start();
  assert.equal(f.queries.length, 0);
  assert.equal(f.commands.length, 1);
  assert.equal(f.commands[0].params.limit, 80);
  assert.deepEqual(f.commands[0].params.range, { start: null, end: null });
  assert.equal(f.state.orders.length, 80);
  assert.equal(f.commands[0].params.query, undefined);
  assert.equal(f.commands[0].params.navigation, undefined);
  assert.equal(f.state.hasMore, true);
  await tick();
  assert.equal(
    f.queries.length,
    0,
    "The initial snapshot must not drain the market",
  );
  assert.equal(
    f.commands.length,
    1,
    "Do not replace the initial coverage immediately",
  );
});
test("empty filtered seed coverage continues without declaring the market empty", async (t) => {
  const f = fixture(t);
  f.skipSeed();
  await f.start();
  assert.equal(f.queries.length, 0);
  assert.equal(f.state.orders.length, 0);
  assert.equal(f.state.hasMore, true);
  t.mock.timers.tick(80);
  await until(() => f.state.orders.length === 100 && !f.state.loadingMore);
  assert.equal(f.queries[0].range!.start, key(1159));
});
test("pagination preserves every loaded row and uses the exclusive continuation", async (t) => {
  const f = fixture(t);
  await f.start();
  f.feed.loadMore();
  await until(() => f.state.orders.length === 180);
  assert.equal(f.queries[0].range!.start, key(1159));
  f.feed.loadMore();
  await until(() => f.state.orders.length === 250);
  assert.equal(f.state.hasMore, false);
  assert.equal(new Set(f.state.orders.map((r) => r.id)).size, 250);
  f.feed.loadMore();
  await tick();
  assert.equal(f.queries.length, 2);
});
test("scrolling cached rows changes watch bounds without querying or dropping pages", async (t) => {
  const f = fixture(t);
  await f.start();
  f.feed.loadMore();
  await until(() => f.state.orders.length === 180);
  f.feed.setVisibleRange(120, 140, 120);
  t.mock.timers.tick(80);
  await tick();
  assert.equal(f.queries.length, 1);
  assert.equal(f.commands.at(-1).params.range.start, row(100).key);
  assert.equal(f.state.orders.length, 180);
  f.feed.setVisibleRange(0, 20, 0);
  t.mock.timers.tick(80);
  await tick();
  assert.equal(f.commands.at(-1).params.range.start, null);
  assert.equal(f.state.orders.length, 180);
});
test("a bounded tail watch does not discard the global exclusive continuation", async (t) => {
  const f = fixture(t);
  await f.start();
  f.feed.loadMore();
  await until(() => f.state.orders.length === 180);
  f.feed.setVisibleRange(170, 179, 170);
  t.mock.timers.tick(80);
  await tick();
  assert.equal(f.commands.at(-1).params.range.end, row(179).key);
  f.feed.loadMore();
  await until(() => f.state.orders.length === 250);
  assert.equal(f.queries[1].range!.start, key(1359));
});
test("live additions and removals reconcile only the covered range", async (t) => {
  const f = fixture(t);
  await f.start();
  const added = { ...row(999), id: "new", key: key(1) };
  f.setRows([added, ...f.rows.filter((r) => r.id !== "5" && r.id !== "120")]);
  f.deliver();
  await tick();
  assert.equal(
    f.state.orders.some((r) => r.id === "new"),
    true,
  );
  assert.equal(
    f.state.orders.some((r) => r.id === "5"),
    false,
  );
  assert.equal(
    f.state.displayOrders.some((r) => r.id === "5"),
    true,
    "Removed cards retain their disappearance state",
  );
  assert.equal(
    f.state.activity.get(JSON.stringify(["v", "new"]))?.state,
    "new",
  );
});
test("reconnect replaces watched state and retains all previously loaded pages", async (t) => {
  const f = fixture(t);
  await f.start();
  f.feed.loadMore();
  await until(() => f.state.orders.length === 180);
  f.reconnect();
  await tick();
  assert.equal(f.queries.length, 1);
  assert.equal(f.state.orders.length, 180);
  assert.equal(f.state.activity.size, 0);
});
test("a burst beyond watch capacity re-seeds coverage without a query", async (t) => {
  const f = fixture(t);
  await f.start();
  f.setRows([
    ...Array.from({ length: 30 }, (_, i) => ({
      ...row(i),
      id: `new${i}`,
      key: key(i + 1),
    })),
    ...f.rows,
  ]);
  f.deliver();
  await until(() => !f.state.error && !f.state.loadingMore);
  assert.equal(f.queries.length, 0, "Capacity recovery re-seeds with a watch");
  assert.equal(
    f.commands.length,
    1,
    "An identical watch is retried with a fresh operation ID",
  );
  assert.equal(
    f.state.orders.some((r) => r.id === "new0"),
    true,
  );
});
test("transient pagination failure retries the current range with bounded backoff", async (t) => {
  const f = fixture(t);
  await f.start();
  f.fail(new ApiFailure("unavailable", "offline"));
  f.feed.loadMore();
  await until(() => !!f.state.error);
  assert.equal(f.queries.length, 1);
  t.mock.timers.tick(2000);
  await until(() => f.state.orders.length === 180 && !f.state.error);
  assert.equal(f.queries.length, 2);
});
test("closing cancels pending pagination and ignores its late result", async (t) => {
  const f = fixture(t);
  await f.start();
  let finish!: () => void;
  f.hold(new Promise<void>((r) => (finish = r)));
  f.feed.loadMore();
  await until(() => f.signals.length === 1);
  const old = f.state;
  f.feed.close();
  assert.equal(f.signals[0].aborted, true);
  finish();
  await tick();
  assert.equal(f.state, old);
  assert.equal(f.commands.length, 1);
});
test("paused range queries reuse and release their snapshot without starting watches", async (t) => {
  const f = fixture(t, true);
  await f.start();
  assert.equal(f.commands.length, 0);
  f.feed.loadMore();
  await until(() => f.state.orders.length === 200);
  assert.equal(f.queries[1].view?.kind, "snapshot");
  assert.equal((f.queries[1].view as any).id, "lease");
  f.feed.close();
  await tick();
  assert.deepEqual(f.releases, ["lease"]);
});
test("unchanged watch bounds preserve sequence continuity", (t) => {
  let receive: (frame: any) => void = () => {};
  t.mock.method(liveSocket, "subscribe", (_q, callback) => {
    receive = callback;
    return () => {};
  });
  t.mock.method(liveSocket, "update", () => false);
  const results: any[] = [];
  const source = new LiveRanges({
    result: (r) => results.push(r),
    reset: () => {},
    status: () => {},
    error: (e) => assert.fail(String(e)),
  });
  t.after(() => source.close());
  const q = {
    ...query,
    source: "asks",
    range: { start: null, end: null },
    limit: 100,
  } as OrderQuery;
  source.watch(q);
  receive({
    type: "orders.snapshot",
    id: "offers",
    sequence: 0,
    next: null,
    data: [row(0)],
    range: q.range,
  });
  source.watch(q);
  receive({
    type: "orders.changed",
    id: "offers",
    sequence: 1,
    upserts: [row(1)],
    removed: [],
  });
  assert.equal(results.at(-1).data.length, 2);
});

test("watch overflow during pagination schedules recovery after the pending read", async (t) => {
  const f = fixture(t);
  await f.start();
  let finish!: () => void;
  f.hold(
    new Promise<void>((resolve) => {
      finish = resolve;
    }),
  );
  f.feed.loadMore();
  await until(() => f.queries.length === 1);
  f.setRows([
    ...Array.from({ length: 30 }, (_, i) => ({
      ...row(i),
      id: `new${i}`,
      key: key(i + 1),
    })),
    ...f.rows,
  ]);
  f.deliver();
  assert.equal(f.queries.length, 1);
  f.hold();
  finish();
  await until(() => !f.state.loadingMore);
  assert.equal(f.queries.length, 1);
  assert.equal(f.state.error, "");
  assert.equal(
    f.state.orders.some((r) => r.id === "new0"),
    true,
  );
});

test("history continues by range and rebuilds frozen pages after lease expiry", async (t) => {
  let state: FeedState,
    expired = false;
  const bodies: any[] = [],
    released: string[] = [];
  const rows = Array.from({ length: 220 }, (_, i) => row(i));
  t.mock.method(globalThis, "fetch", async (_path: any, init: RequestInit) => {
    const body = JSON.parse(String(init.body));
    if (init.method === "DELETE") {
      released.push(body.snapshot);
      return new Response(null, { status: 204 });
    }
    bodies.push(body);
    if (!expired && body.view.id === "lease1" && body.range.start) {
      expired = true;
      return new Response(
        JSON.stringify({ error: { code: "restart_read", message: "expired" } }),
        { status: 409 },
      );
    }
    const result = page(body, rows);
    return new Response(
      JSON.stringify({
        ...result,
        data: result.data.map((r) => ({
          ...r,
          kind: "executed",
          executed_at: r.observed_at,
          price: String(Number(r.price) + (expired ? 10000 : 0)),
        })),
        snapshot: expired ? "lease2" : "lease1",
      }),
    );
  });
  const feed = createOrderFeed(
    {
      kind: "history",
      query: { filters: query.filters, sort: "time_desc", limit: 100 },
    },
    false,
    (next) => {
      state = next;
    },
  );
  t.after(() => feed.close());
  feed.start();
  await until(() => feed.ready);
  assert.equal(state!.orders.length, 100);
  feed.loadMore();
  await until(() => state!.orders.length === 200 && !state!.loadingMore);
  assert.equal(bodies[1].range.start, key(1199));
  assert.equal(
    state!.orders.every((r) => Number(r.price) >= 10001),
    true,
    "Expired and fresh snapshots must not be mixed",
  );
  assert.equal(new Set(state!.orders.map((r) => r.id)).size, 200);
  feed.loadMore();
  await until(() => state!.orders.length === 220);
  assert.equal(state!.hasMore, false);
  feed.close();
  await tick();
  assert.deepEqual(released, ["lease1", "lease2"]);
  assert.equal(
    bodies.every((q) => q.limit === 100 && q.navigation === undefined),
    true,
  );
});

test("closed history releases a late snapshot without publishing stale rows", async (t) => {
  let finish!: () => void, state: FeedState;
  const pending = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const released: string[] = [];
  t.mock.method(globalThis, "fetch", async (_path: any, init: RequestInit) => {
    if (init.method === "DELETE") {
      released.push(JSON.parse(String(init.body)).snapshot);
      return new Response(null, { status: 204 });
    }
    await pending;
    return new Response(
      JSON.stringify({
        ...page(
          { range: { start: null, end: null }, limit: 100 } as OrderQuery,
          [row(0)],
        ),
        snapshot: "late",
      }),
    );
  });
  const feed = createOrderFeed(
    { kind: "history", query: { filters: query.filters, limit: 100 } },
    false,
    (next) => {
      state = next;
    },
  );
  feed.start();
  const before = state!;
  feed.close();
  finish();
  await tick();
  await tick();
  assert.equal(state!, before);
  assert.deepEqual(released, ["late"]);
});
