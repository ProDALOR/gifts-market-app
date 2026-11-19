import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { LiveSocket } from "../src/live-socket.ts";
import { ApiFailure } from "../src/api-error.ts";
function setup(t: TestContext) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  class Socket {
    static all: Socket[] = [];
    static OPEN = 1;
    readyState = 0;
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
    open() {
      this.readyState = 1;
      this.onopen?.();
    }
    close() {
      this.readyState = 3;
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
  type: "catalog.watch" as const,
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
  first.open();
  const initial = first.commands("catalog.watch")[0].id;
  const stop = live.subscribe(catalog(), (frame) => {
    assert.equal(frame.id, "catalog");
    updates++;
  });
  old();
  const replaced = first.commands("catalog.watch").at(-1).id;
  assert.notEqual(initial, replaced);
  first.receive({ type: "catalog.versions", id: initial, versions });
  assert.equal(updates, 0);
  first.receive({ type: "catalog.versions", id: replaced, versions });
  assert.equal(updates, 1);
  const stopOrders = live.subscribe(catalog("orders"), () => {});
  live.update({ ...catalog(), collection_ids: ["other"] });
  const updated = first.commands("catalog.watch").at(-1).id;
  assert.notEqual(updated, replaced);
  const count = first.sent.length;
  live.update({ ...catalog(), collection_ids: ["other"] });
  assert.equal(first.sent.length, count);
  stopOrders();
  assert.equal(first.closed, false);
  first.close();
  t.mock.timers.tick(1000);
  const second = Socket.all[1];
  second.open();
  const reconnected = second.commands("catalog.watch")[0].id;
  assert.notEqual(updated, reconnected);
  second.receive({ type: "catalog.versions", id: updated, versions });
  assert.equal(updates, 1);
  first.receive({ type: "catalog.versions", id: reconnected, versions });
  assert.equal(updates, 1);
  second.receive({ type: "catalog.versions", id: reconnected, versions });
  assert.equal(updates, 2);
  stop();
  assert.equal(second.closed, true);
  t.mock.timers.tick(10000);
  assert.equal(Socket.all.length, 2);
});

test("terminal errors retire old identities and retry only their subscription", (t) => {
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
  socket.open();
  const current = () =>
    socket
      .commands("catalog.watch")
      .filter((c) => c.collection_ids[0] === "gift")[0];
  const fail = (code: string) =>
    socket.receive({
      type: "error",
      id: socket.commands("catalog.watch").at(-1).id,
      error: { code, message: "Failed" },
    });
  // Catalog is the first operation, other remains healthy.
  socket.receive({
    type: "error",
    id: current().id,
    error: { code: "capacity_exceeded", message: "Busy" },
  });
  t.mock.timers.tick(1000);
  assert.equal(socket.commands("catalog.watch").length, 3);
  fail("unavailable");
  t.mock.timers.tick(1999);
  assert.equal(socket.commands("catalog.watch").length, 3);
  t.mock.timers.tick(1);
  assert.equal(socket.commands("catalog.watch").length, 4);
  fail("invalid_query");
  t.mock.timers.tick(15000);
  assert.equal(socket.commands("catalog.watch").length, 4);
  live.retry("catalog");
  assert.equal(socket.commands("catalog.watch").length, 5);
  fail("unavailable");
  socket.receive({
    type: "catalog.versions",
    id: socket.commands("catalog.watch").at(-1).id,
    versions,
  });
  t.mock.timers.tick(1000);
  assert.equal(
    socket.commands("catalog.watch").length,
    6,
    "late terminal frames cannot cancel retries",
  );
  socket.receive({
    type: "catalog.versions",
    id: socket.commands("catalog.watch").at(-1).id,
    versions,
  });
  t.mock.timers.tick(15000);
  assert.equal(socket.commands("catalog.watch").length, 6);
  assert.equal(
    socket.commands("cancel").length,
    0,
    "terminal operations need no cancellation",
  );
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
  socket.open();
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
  Socket.all[0].open();
  assert.equal(Socket.all[0].sent.length, 0);
});

test("disconnect rejects requests once and reconnects only subscriptions", async (t) => {
  const { Socket, live } = setup(t);
  const stop = live.subscribe(catalog(), () => {});
  t.after(stop);
  const promise = live.request(call);
  const rejected = assert.rejects(promise, { code: "unavailable" });
  const first = Socket.all[0];
  first.open();
  first.close();
  await rejected;
  t.mock.timers.tick(1000);
  const second = Socket.all[1];
  second.open();
  assert.equal(second.commands("request").length, 0);
  assert.equal(second.commands("catalog.watch").length, 1);
});

test("stalled handshakes expire, and releasing the last observer cancels reconnects", async (t) => {
  const { Socket, live } = setup(t);
  const stop = live.watchStatus(() => {});
  const pending = live.request(call);
  const rejected = assert.rejects(pending, { code: "unavailable" });
  const first = Socket.all[0];
  t.mock.timers.tick(9999);
  assert.equal(first.closed, false);
  t.mock.timers.tick(1);
  await rejected;
  assert.equal(first.closed, true);
  first.open();
  assert.equal(first.sent.length, 0, "retired socket callbacks are detached");
  t.mock.timers.tick(1000);
  const second = Socket.all[1];
  second.open();
  assert.equal(
    second.commands("request").length,
    0,
    "requests are not replayed",
  );
  t.mock.timers.tick(10000);
  assert.equal(second.closed, false, "opening clears the handshake deadline");
  stop();
  assert.equal(second.closed, true);
  t.mock.timers.tick(60000);
  assert.equal(Socket.all.length, 2);
});

test("cancelling requests queued during reconnect sends no stale cancellation", async (t) => {
  const { Socket, live } = setup(t);
  const stop = live.subscribe(catalog(), () => {});
  const first = Socket.all[0];
  first.open();
  first.close();
  const abort = new AbortController();
  const pending = live.request(call, abort.signal);
  const rejected = assert.rejects(pending, { name: "AbortError" });
  abort.abort();
  await rejected;
  t.mock.timers.tick(1000);
  const second = Socket.all[1];
  second.open();
  assert.equal(second.commands("request").length, 0);
  assert.equal(second.commands("cancel").length, 0);
  assert.equal(second.commands("catalog.watch").length, 1);
  stop();
});

test("pending request budget and timeout release all work", async (t) => {
  const { Socket, live } = setup(t);
  const promises = Array.from({ length: 8 }, () => live.request(call));
  const rejected = promises.map((p) =>
    assert.rejects(p, { code: "timed_out" }),
  );
  await assert.rejects(live.request(call), { code: "capacity_exceeded" });
  const socket = Socket.all[0];
  socket.open();
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
  socket.open();
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
  socket.open();
  socket.receive(result(socket.sent[0].id));
  assert.equal(socket.closed, true);
});

test("unsubscribing inside an error callback leaves no delayed retry", (t) => {
  const { Socket, live } = setup(t);
  let stop = () => {};
  stop = live.subscribe(catalog(), () => stop());
  const socket = Socket.all[0];
  socket.open();
  socket.receive({
    type: "error",
    id: socket.sent[0].id,
    error: { code: "unavailable", message: "Stopped" },
  });
  t.mock.timers.tick(60000);
  assert.equal(socket.closed, true);
  assert.equal(Socket.all.length, 1);
  assert.equal(socket.commands("catalog.watch").length, 1);
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
  first.open();
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
  second.open();
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
  socket.open();
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

test("malformed command rejections leave pending requests and subscriptions running", async (t) => {
  const { Socket, live } = setup(t);
  let updates = 0,
    rejections = 0;
  const stop = live.subscribe(catalog(), () => updates++);
  live.onCommandError = (frame) => {
    assert.equal(frame.id, "");
    rejections++;
  };
  const pending = live.request(call);
  const socket = Socket.all[0];
  socket.open();
  socket.receive({
    type: "command_error",
    id: "",
    error: { code: "invalid_query", message: "Invalid command" },
  });
  assert.equal(socket.closed, false);
  assert.equal(rejections, 1);
  assert.equal(updates, 0);
  socket.receive({
    type: "catalog.versions",
    id: socket.commands("catalog.watch")[0].id,
    versions,
  });
  socket.receive(result(socket.commands("request")[0].id));
  assert.equal(updates, 1);
  assert.deepEqual(await pending, report);
  assert.equal(socket.commands("cancel").length, 0);
  stop();
});

test("retrying inside a terminal error callback starts one fresh operation", (t) => {
  const { Socket, live } = setup(t);
  const stop = live.subscribe(catalog(), (frame) => {
    if (frame.type === "error") live.retry("catalog");
  });
  const socket = Socket.all[0];
  socket.open();
  const old = socket.commands("catalog.watch")[0].id;
  socket.receive({
    type: "error",
    id: old,
    error: { code: "unavailable", message: "Retry" },
  });
  const current = socket.commands("catalog.watch").at(-1).id;
  assert.notEqual(current, old);
  assert.equal(socket.commands("cancel").length, 0);
  t.mock.timers.tick(60000);
  assert.equal(socket.commands("catalog.watch").length, 2);
  stop();
});

test("connection errors retire old work before error callbacks queue replacements", async (t) => {
  const { Socket, live } = setup(t);
  let recovery: Promise<unknown> | undefined;
  const stop = live.subscribe(catalog(), (frame) => {
    if (frame.type === "error") recovery = live.request(call);
  });
  const old = live.request(call);
  const rejected = assert.rejects(old, { code: "timed_out" });
  const first = Socket.all[0];
  first.open();
  first.receive({
    type: "error",
    id: "",
    error: { code: "timed_out", message: "Connection failed" },
  });
  await rejected;
  assert.ok(recovery);
  assert.equal(Socket.all.length, 1);
  t.mock.timers.tick(1000);
  const second = Socket.all[1];
  second.open();
  assert.equal(second.commands("request").length, 1);
  second.receive(result(second.commands("request")[0].id));
  assert.deepEqual(await recovery, report);
  stop();
});

test("one connection status stream is shared, replayed and released independently", (t) => {
  const { Socket, live } = setup(t);
  const states: string[] = [];
  const a = live.watchStatus((status) => states.push(status));
  const socket = Socket.all[0];
  socket.open();
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

test("disconnect callbacks can queue requests and replace subscriptions without losing new work", async (t) => {
  const { Socket, live } = setup(t);
  let recovered: Promise<unknown> | undefined;
  let stopReplacement = () => {};
  let updates = 0;
  const stopOld = live.subscribe(catalog(), () => assert.fail("Old listener"));
  let interrupted = false;
  const stopStatus = live.watchStatus((status) => {
    if (status !== "unavailable" || !interrupted) return;
    interrupted = false;
    recovered = live.request(call);
    stopReplacement = live.subscribe(catalog(), () => updates++);
  });
  const oldRequest = live.request(call);
  const rejected = assert.rejects(oldRequest, { code: "unavailable" });
  const first = Socket.all[0];
  first.open();
  first.receive({ type: "market.status", status: "available" });
  interrupted = true;
  first.close();
  await rejected;
  assert.ok(recovered);
  assert.equal(
    Socket.all.length,
    1,
    "callbacks must respect reconnect backoff",
  );
  t.mock.timers.tick(1000);
  assert.equal(Socket.all.length, 2);
  const second = Socket.all[1];
  second.open();
  second.receive({ type: "market.status", status: "available" });
  const request = second.commands("request")[0];
  assert.ok(request, "new request must survive old-connection cleanup");
  assert.equal(second.commands("request").length, 1);
  assert.equal(second.commands("catalog.watch").length, 1);
  first.receive({ type: "market.status", status: "delayed" });
  first.receive(result(request.id));
  assert.equal(live.statusSnapshot().status, "available");
  second.receive(result(request.id));
  assert.deepEqual(await recovered, report);
  second.receive({
    type: "catalog.versions",
    id: second.commands("catalog.watch")[0].id,
    versions,
  });
  assert.equal(updates, 1);
  stopOld();
  stopReplacement();
  stopStatus();
  t.mock.timers.tick(60000);
  assert.equal(Socket.all.length, 2);
  assert.equal(second.closed, true);
});

test("a successful status-only connection resets reconnect backoff", (t) => {
  const { Socket, live } = setup(t);
  const stop = live.watchStatus(() => {});
  Socket.all[0].close();
  t.mock.timers.tick(1000);
  Socket.all[1].close();
  t.mock.timers.tick(2000);
  const connected = Socket.all[2];
  connected.open();
  connected.receive({ type: "market.status", status: "delayed" });
  connected.close();
  t.mock.timers.tick(999);
  assert.equal(Socket.all.length, 3);
  t.mock.timers.tick(1);
  assert.equal(Socket.all.length, 4);
  stop();
});

test("a market interruption invalidates old estimates through recovery without extra requests", async (t) => {
  const { estimateIsStale } = await import("../src/valuation/freshness.ts");
  const { Socket, live } = setup(t);
  const unsubscribe = live.watchStatus(() => {});
  const socket = Socket.all[0];
  socket.open();
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
