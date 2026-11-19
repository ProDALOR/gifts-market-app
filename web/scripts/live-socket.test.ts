import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { LiveSocket } from "../src/live-socket.ts";
import { ApiFailure } from "../src/api-error.ts";
function setup(t: TestContext) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  class Socket {
    static all: Socket[] = [];
    sent: any[] = [];
    closed = false;
    onopen?: () => void;
    onclose?: () => void;
    onerror?: () => void;
    onmessage?: (event: { data: string }) => void;
    constructor() {
      Socket.all.push(this);
    }
    send(value: string) {
      this.sent.push(JSON.parse(value));
    }
    close() {
      this.closed = true;
      this.onclose?.();
    }
    receive(value: unknown) {
      this.onmessage?.({ data: JSON.stringify(value) });
    }
    commands(type: string) {
      return this.sent.filter((value) => value.type === type);
    }
  }
  for (const [name, value] of Object.entries({
    WebSocket: Socket,
    location: { protocol: "https:", host: "test" },
  })) {
    const previous = Object.getOwnPropertyDescriptor(globalThis, name);
    Object.defineProperty(globalThis, name, { configurable: true, value });
    t.after(() =>
      previous
        ? Object.defineProperty(globalThis, name, previous)
        : Reflect.deleteProperty(globalThis, name),
    );
  }
  return { Socket, live: new LiveSocket(() => 0.5) };
}
const catalog = (id = "catalog") => ({
  type: "watch_catalog" as const,
  id,
  collection_ids: ["gift"],
});
const versions = { directory: "d", collections: { gift: "v" } };
const call = {
  method: "valuation.evaluate" as const,
  params: { gift: "Gift-1", currency_id: "GRAM" },
};
const report = {
  strategy: "gift_floor",
  evaluated_at: "2026-11-18T12:00:00Z",
  currency_id: "GRAM",
  gift: {
    id: "1",
    collection_id: "gift",
    number: 1,
    traits: { model: null, backdrop: null, pattern: null },
  },
  stale: false,
  display: { fields: [], summary: null, note: "Current asks" },
};
const result = (id: string) => ({
  type: "result",
  id,
  method: call.method,
  data: report,
});

test("replacement, updates and reconnects use fresh identities and ignore delayed frames", (t) => {
  const { Socket, live } = setup(t);
  let updates = 0;
  const old = live.subscribe(catalog(), () =>
    assert.fail("Replaced subscriber"),
  );
  const first = Socket.all[0];
  first.onopen?.();
  const initial = first.commands("watch_catalog")[0].id;
  const stop = live.subscribe(catalog(), (frame) => {
    assert.equal(frame.id, "catalog");
    updates++;
  });
  old();
  const replaced = first.commands("watch_catalog").at(-1).id;
  assert.notEqual(initial, replaced);
  first.receive({ type: "catalog", id: initial, versions });
  assert.equal(updates, 0);
  first.receive({ type: "catalog", id: replaced, versions });
  assert.equal(updates, 1);
  const stopOrders = live.subscribe(catalog("orders"), () => {});
  live.update({ ...catalog(), collection_ids: ["other"] });
  const updated = first.commands("watch_catalog").at(-1).id;
  assert.notEqual(updated, replaced);
  const count = first.sent.length;
  live.update({ ...catalog(), collection_ids: ["other"] });
  assert.equal(first.sent.length, count);
  stopOrders();
  assert.equal(first.closed, false);
  first.close();
  t.mock.timers.tick(1000);
  const second = Socket.all[1];
  second.onopen?.();
  const reconnected = second.commands("watch_catalog")[0].id;
  assert.notEqual(updated, reconnected);
  second.receive({ type: "catalog", id: updated, versions });
  assert.equal(updates, 1);
  first.receive({ type: "catalog", id: reconnected, versions });
  assert.equal(updates, 1);
  second.receive({ type: "catalog", id: reconnected, versions });
  assert.equal(updates, 2);
  stop();
  assert.equal(second.closed, true);
  t.mock.timers.tick(10000);
  assert.equal(Socket.all.length, 2);
});

test("transient errors retry only their subscription and valid frames clear stale retries", (t) => {
  const { Socket, live } = setup(t),
    errors: string[] = [];
  const stop = live.subscribe(catalog(), (f) => {
    if (f.type === "error") errors.push(f.error.code);
  });
  const other = live.subscribe(catalog("other"), () => {});
  t.after(() => {
    stop();
    other();
  });
  const socket = Socket.all[0];
  socket.onopen?.();
  const current = () =>
    socket
      .commands("watch_catalog")
      .filter((c) => c.collection_ids[0] === "gift")[0];
  const fail = (code: string) =>
    socket.receive({
      type: "error",
      id: socket.commands("watch_catalog").at(-1).id,
      error: { code, message: "Failed" },
    });
  // Catalog is the first operation, other remains healthy.
  socket.receive({
    type: "error",
    id: current().id,
    error: { code: "capacity_exceeded", message: "Busy" },
  });
  t.mock.timers.tick(1000);
  assert.equal(socket.commands("watch_catalog").length, 3);
  fail("unavailable");
  t.mock.timers.tick(1999);
  assert.equal(socket.commands("watch_catalog").length, 3);
  t.mock.timers.tick(1);
  assert.equal(socket.commands("watch_catalog").length, 4);
  fail("invalid_query");
  t.mock.timers.tick(15000);
  assert.equal(socket.commands("watch_catalog").length, 4);
  live.retry("catalog");
  assert.equal(socket.commands("watch_catalog").length, 5);
  fail("unavailable");
  socket.receive({
    type: "catalog",
    id: socket.commands("watch_catalog").at(-1).id,
    versions,
  });
  t.mock.timers.tick(15000);
  assert.equal(socket.commands("watch_catalog").length, 5);
  assert.equal(errors.includes("invalid_query"), true);
  assert.equal(socket.closed, false);
});

test("requests resolve out of order and cancellation ignores late results without interrupting subscriptions", async (t) => {
  const { Socket, live } = setup(t);
  const stop = live.subscribe(catalog(), () => {});
  t.after(stop);
  const abort = new AbortController();
  const one = live.request(call, abort.signal),
    two = live.request(call);
  const rejected = assert.rejects(one, { name: "AbortError" });
  const socket = Socket.all[0];
  socket.onopen?.();
  const [a, b] = socket.commands("request");
  socket.receive(result(b.id));
  assert.deepEqual(await two, report);
  abort.abort();
  await rejected;
  assert.equal(socket.commands("cancel").at(-1).id, a.id);
  socket.receive(result(a.id));
  assert.equal(socket.closed, false);
  const three = live.request(call);
  const c = socket.commands("request").at(-1);
  socket.receive({
    type: "error",
    id: c.id,
    error: {
      code: "not_found",
      message: "Missing",
      details: { strategy: "gift_floor" },
    },
  });
  await assert.rejects(three, (e: unknown) => {
    assert.ok(e instanceof ApiFailure);
    assert.equal(e.code, "not_found");
    assert.deepEqual(e.details, { strategy: "gift_floor" });
    return true;
  });
  assert.equal(Socket.all.length, 1);
});

test("queued cancellation sends no request and an idle socket closes", async (t) => {
  const { Socket, live } = setup(t),
    abort = new AbortController();
  const promise = live.request(call, abort.signal);
  const rejected = assert.rejects(promise, { name: "AbortError" });
  abort.abort();
  await rejected;
  assert.equal(Socket.all[0].closed, true);
  assert.equal(Socket.all[0].sent.length, 0);
  Socket.all[0].onopen?.();
  assert.equal(Socket.all[0].sent.length, 0);
});

test("disconnect rejects requests once and reconnects only subscriptions", async (t) => {
  const { Socket, live } = setup(t);
  const stop = live.subscribe(catalog(), () => {});
  t.after(stop);
  const promise = live.request(call);
  const rejected = assert.rejects(promise, { code: "unavailable" });
  const first = Socket.all[0];
  first.onopen?.();
  first.close();
  await rejected;
  t.mock.timers.tick(1000);
  const second = Socket.all[1];
  second.onopen?.();
  assert.equal(second.commands("request").length, 0);
  assert.equal(second.commands("watch_catalog").length, 1);
});

test("pending request budget and timeout release all work", async (t) => {
  const { Socket, live } = setup(t);
  const promises = Array.from({ length: 8 }, () => live.request(call));
  const rejected = promises.map((p) =>
    assert.rejects(p, { code: "timed_out" }),
  );
  await assert.rejects(live.request(call), { code: "capacity_exceeded" });
  const socket = Socket.all[0];
  socket.onopen?.();
  t.mock.timers.tick(35000);
  await Promise.all(rejected);
  assert.equal(socket.commands("cancel").length, 8);
  assert.equal(socket.closed, true);
});

test("malformed frames fail pending requests and recover subscriptions without throwing", async (t) => {
  const { Socket, live } = setup(t);
  const stop = live.subscribe(catalog(), () => {});
  t.after(stop);
  const promise = live.request(call);
  const rejected = assert.rejects(promise, { code: "unavailable" });
  const socket = Socket.all[0];
  socket.onopen?.();
  socket.receive({ type: "page", id: "op.1", page: { data: null } });
  await rejected;
  assert.equal(socket.closed, true);
  t.mock.timers.tick(1000);
  assert.equal(Socket.all.length, 2);
});

test("a valid frame for the wrong operation kind is rejected", (t) => {
  const { Socket, live } = setup(t);
  const stop = live.subscribe(catalog(), () => assert.fail("Wrong channel"));
  t.after(stop);
  const socket = Socket.all[0];
  socket.onopen?.();
  socket.receive({
    type: "sync",
    id: socket.sent[0].id,
    sync: report.sync,
  });
  assert.equal(socket.closed, true);
});

test("unsubscribing inside an error callback leaves no delayed retry", (t) => {
  const { Socket, live } = setup(t);
  let stop = () => {};
  stop = live.subscribe(catalog(), () => stop());
  const socket = Socket.all[0];
  socket.onopen?.();
  socket.receive({
    type: "error",
    id: socket.sent[0].id,
    error: { code: "unavailable", message: "Stopped" },
  });
  t.mock.timers.tick(60000);
  assert.equal(socket.closed, true);
  assert.equal(Socket.all.length, 1);
  assert.equal(socket.commands("watch_catalog").length, 1);
});

test("resume anchors do not suppress navigation and reconnect uses the latest visible position", (t) => {
  const { Socket, live } = setup(t);
  const command = {
    type: "orders.watch" as const,
    id: "offers",
    params: {
      query: {
        filters: { collection_id: "gift", currency_id: "GRAM" },
        position: null as string | null,
        limit: 150,
      },
    },
  };
  const stop = live.subscribe(command, () => {});
  t.after(stop);
  const first = Socket.all[0];
  first.onopen?.();
  const next = {
    ...command,
    params: { query: { ...command.params.query, position: "visible" } },
  };
  live.remember(next);
  assert.equal(first.commands("orders.watch").length, 1);
  live.update(next);
  assert.equal(first.commands("orders.watch").length, 2);
  live.remember({
    ...next,
    params: { query: { ...next.params.query, position: "latest" } },
  });
  first.close();
  t.mock.timers.tick(1000);
  const second = Socket.all[1];
  second.onopen?.();
  assert.equal(
    second.commands("orders.watch")[0].params.query.position,
    "latest",
  );
});

test("command rejections do not complete or cancel the original request", async (t) => {
  const { Socket, live } = setup(t);
  let rejected = 0;
  live.onCommandError = () => rejected++;
  const promise = live.request(call);
  const socket = Socket.all[0];
  socket.onopen?.();
  const id = socket.commands("request")[0].id;
  socket.receive({
    type: "command_error",
    id,
    error: { code: "invalid_query", message: "duplicate operation ID" },
  });
  assert.equal(rejected, 1);
  assert.equal(socket.commands("cancel").length, 0);
  assert.equal(socket.closed, false);
  socket.receive(result(id));
  assert.deepEqual(await promise, report);
});

test("one connection status stream is shared, replayed and released independently", (t) => {
  const { Socket, live } = setup(t);
  const states: string[] = [];
  const a = live.watchStatus((status) => states.push(status));
  const socket = Socket.all[0];
  socket.onopen?.();
  socket.receive({
    type: "market.status",
    status: "available",
  });
  const b = live.watchStatus((status) => states.push(`b:${status}`));
  assert.equal(Socket.all.length, 1);
  assert.deepEqual(states, ["unavailable", "available", "b:available"]);
  a();
  assert.equal(socket.closed, false);
  socket.onerror?.();
  assert.equal(states.at(-1), "b:unavailable");
  t.mock.timers.tick(1000);
  assert.equal(Socket.all.length, 2);
  b();
  assert.equal(Socket.all[1].closed, true);
  t.mock.timers.tick(60000);
  assert.equal(Socket.all.length, 2);
});

test("a market interruption invalidates old estimates through recovery without extra requests", async (t) => {
  const { estimateIsStale } = await import("../src/valuation/freshness.ts");
  const { Socket, live } = setup(t);
  const unsubscribe = live.watchStatus(() => {});
  const socket = Socket.all[0];
  socket.onopen?.();
  const publish = (status: string) =>
    socket.receive({ type: "market.status", status });
  publish("available");
  const evaluated = live.statusSnapshot();
  assert.equal(
    estimateIsStale({ stale: false }, evaluated.generation, evaluated),
    false,
  );
  publish("available");
  assert.equal(
    live.statusSnapshot(),
    evaluated,
    "duplicate availability must not rerender subscribers",
  );
  publish("delayed");
  assert.equal(
    estimateIsStale(
      { stale: false },
      evaluated.generation,
      live.statusSnapshot(),
    ),
    true,
  );
  publish("available");
  assert.equal(
    estimateIsStale(
      { stale: false },
      evaluated.generation,
      live.statusSnapshot(),
    ),
    true,
    "recovery cannot make an old estimate current",
  );
  const refreshed = live.statusSnapshot().generation;
  assert.equal(
    estimateIsStale({ stale: false }, refreshed, live.statusSnapshot()),
    false,
  );
  assert.equal(
    estimateIsStale({ stale: true }, refreshed, live.statusSnapshot()),
    true,
  );
  socket.onerror?.();
  assert.equal(
    estimateIsStale({ stale: false }, refreshed, live.statusSnapshot()),
    true,
  );
  assert.equal(socket.commands("request").length, 0);
  unsubscribe();
});
