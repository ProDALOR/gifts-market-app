import assert from "node:assert/strict";
import { test } from "node:test";
import { LiveSocket } from "../src/live-socket.ts";
test("orders and catalog share one socket, update subscriptions and reconnect together", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  class Socket {
    static all: Socket[] = [];
    sent: string[] = [];
    closed = false;
    onopen?: () => void;
    onclose?: () => void;
    onerror?: () => void;
    onmessage?: (event: { data: string }) => void;
    constructor() {
      Socket.all.push(this);
    }
    send(value: string) {
      this.sent.push(value);
    }
    close() {
      this.closed = true;
      this.onclose?.();
    }
  }
  const oldSocket = Object.getOwnPropertyDescriptor(globalThis, "WebSocket");
  const oldLocation = Object.getOwnPropertyDescriptor(globalThis, "location");
  Object.defineProperty(globalThis, "WebSocket", {
    configurable: true,
    value: Socket,
  });
  Object.defineProperty(globalThis, "location", {
    configurable: true,
    value: { protocol: "https:", host: "test" },
  });
  t.after(() => {
    if (oldSocket) Object.defineProperty(globalThis, "WebSocket", oldSocket);
    else Reflect.deleteProperty(globalThis, "WebSocket");
    if (oldLocation) Object.defineProperty(globalThis, "location", oldLocation);
    else Reflect.deleteProperty(globalThis, "location");
  });
  const live = new LiveSocket(() => 0.5);
  let updates = 0;
  const catalog = live.subscribe(
    { type: "watch_catalog", id: "catalog", collection_ids: ["gift"] },
    () => updates++,
  );
  const orders = live.subscribe(
    { type: "watch_catalog", id: "offers", collection_ids: [] },
    () => assert.fail("Wrong subscriber"),
  );
  assert.equal(Socket.all.length, 1);
  const first = Socket.all[0];
  first.onopen?.();
  assert.equal(first.sent.length, 2);
  live.update({
    type: "watch_catalog",
    id: "catalog",
    collection_ids: ["gift", "other"],
  });
  assert.deepEqual(
    first.sent.slice(-2).map((s) => JSON.parse(s).type),
    ["unsubscribe", "watch_catalog"],
  );
  const count = first.sent.length;
  live.update({
    type: "watch_catalog",
    id: "catalog",
    collection_ids: ["gift", "other"],
  });
  assert.equal(first.sent.length, count);
  orders();
  assert.equal(
    first.closed,
    false,
    "History and paused views keep the catalog subscription",
  );
  first.close();
  t.mock.timers.tick(1000);
  assert.equal(Socket.all.length, 2);
  const second = Socket.all[1];
  second.onopen?.();
  assert.equal(second.sent.length, 1);
  const event = {
    data: JSON.stringify({
      type: "catalog",
      id: "catalog",
      versions: { directory: "d", collections: {} },
    }),
  };
  first.onmessage?.(event);
  assert.equal(updates, 0);
  second.onmessage?.(event);
  assert.equal(updates, 1);
  catalog();
  assert.equal(second.closed, true);
  t.mock.timers.tick(5000);
  assert.equal(Socket.all.length, 2);
});

test("catalog errors retry only that subscription with backoff and surface permanent failures", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  class Socket {
    static all: Socket[] = [];
    sent: string[] = [];
    closed = false;
    onopen?: () => void;
    onclose?: () => void;
    onerror?: () => void;
    onmessage?: (event: { data: string }) => void;
    constructor() {
      Socket.all.push(this);
    }
    send(value: string) {
      this.sent.push(value);
    }
    close() {
      this.closed = true;
      this.onclose?.();
    }
  }
  const previousSocket = Object.getOwnPropertyDescriptor(
      globalThis,
      "WebSocket",
    ),
    previousLocation = Object.getOwnPropertyDescriptor(globalThis, "location");
  Object.defineProperty(globalThis, "WebSocket", {
    configurable: true,
    value: Socket,
  });
  Object.defineProperty(globalThis, "location", {
    configurable: true,
    value: { protocol: "http:", host: "test" },
  });
  t.after(() => {
    if (previousSocket)
      Object.defineProperty(globalThis, "WebSocket", previousSocket);
    else Reflect.deleteProperty(globalThis, "WebSocket");
    if (previousLocation)
      Object.defineProperty(globalThis, "location", previousLocation);
    else Reflect.deleteProperty(globalThis, "location");
  });
  const live = new LiveSocket(() => 0.5),
    errors: string[] = [];
  const stopCatalog = live.subscribe(
    { type: "watch_catalog", id: "catalog", collection_ids: ["gift"] },
    (frame) => {
      if (frame.type === "error") errors.push(frame.error.code);
    },
  );
  const stopOrders = live.subscribe(
    { type: "watch_catalog", id: "offers", collection_ids: [] },
    () => {},
  );
  t.after(() => {
    stopCatalog();
    stopOrders();
  });
  const socket = Socket.all[0];
  socket.onopen?.();
  const subscriptions = (id: string) =>
    socket.sent
      .map((value) => JSON.parse(value))
      .filter((value) => value.id === id && value.type !== "unsubscribe")
      .length;
  const fail = (code: string) =>
    socket.onmessage?.({
      data: JSON.stringify({
        type: "error",
        id: "catalog",
        error: { code, message: "catalog failed" },
      }),
    });
  fail("capacity_exceeded");
  assert.equal(socket.closed, false);
  t.mock.timers.tick(1000);
  assert.equal(subscriptions("catalog"), 2);
  assert.equal(subscriptions("offers"), 1);
  fail("unavailable");
  t.mock.timers.tick(1999);
  assert.equal(subscriptions("catalog"), 2);
  t.mock.timers.tick(1);
  assert.equal(subscriptions("catalog"), 3);
  fail("invalid_query");
  t.mock.timers.tick(15000);
  assert.equal(subscriptions("catalog"), 3);
  assert.equal(errors.at(-1), "invalid_query");
  live.retry("catalog");
  assert.equal(subscriptions("catalog"), 4);
  assert.equal(subscriptions("offers"), 1);
  fail("unavailable");
  stopCatalog();
  t.mock.timers.tick(15000);
  assert.equal(subscriptions("catalog"), 4);
  assert.equal(socket.closed, false);
  assert.equal(Socket.all.length, 1);
});
