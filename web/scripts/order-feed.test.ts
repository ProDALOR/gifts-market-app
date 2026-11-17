import assert from "node:assert/strict";
import { test } from "node:test";
import type { TestContext } from "node:test";
import { OrderFeed, mergePages } from "../src/order-feed.ts";
import type { FeedState } from "../src/order-feed.ts";
import type { Order, Page, Query } from "../src/market.ts";
const order = (id: string, currency = "GRAM") =>
  ({
    id,
    book: { venue_id: "test", currency_id: currency, item_type_id: "gift" },
    price: id,
    attributes: {},
  }) as Order;
const page = (rows: Order[], next = ""): Page<Order> => ({
  data: rows,
  next_page_token: next,
  cursor: "test",
  sync: { ready: true, state: "live", updated_at: null },
});
const query: Query = {
  scope: { item_type_id: "gift", currency_id: "GRAM", modes: ["ask"] },
  sort: "price_asc",
  consistency: "snapshot",
  limit: 2,
  page_token: "",
};
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
async function until(test: () => boolean) {
  for (let n = 0; n < 100; n++) {
    if (test()) return;
    await tick();
  }
  assert.fail("Feed did not settle");
}
function sockets(t: TestContext) {
  class Socket {
    static opened: Socket[] = [];
    onopen?: () => void;
    onmessage?: (event: { data: string }) => void;
    onclose?: () => void;
    onerror?: () => void;
    constructor() {
      Socket.opened.push(this);
      queueMicrotask(() => this.onopen?.());
    }
    send() {}
    close() {}
    deliver(data: Page<Order>) {
      this.onmessage?.({ data: JSON.stringify({ type: "page", page: data }) });
    }
  }
  const oldSocket = Object.getOwnPropertyDescriptor(globalThis, "WebSocket"),
    oldLocation = Object.getOwnPropertyDescriptor(globalThis, "location");
  Object.defineProperty(globalThis, "WebSocket", {
    configurable: true,
    value: Socket,
  });
  Object.defineProperty(globalThis, "location", {
    configurable: true,
    value: { protocol: "http:", host: "test" },
  });
  t.after(() => {
    if (oldSocket) Object.defineProperty(globalThis, "WebSocket", oldSocket);
    else Reflect.deleteProperty(globalThis, "WebSocket");
    if (oldLocation) Object.defineProperty(globalThis, "location", oldLocation);
    else Reflect.deleteProperty(globalThis, "location");
  });
  return Socket;
}
test("scroll pages deduplicate only identical venue/order identities", () => {
  const other = {
    ...order("a"),
    book: { ...order("a").book, venue_id: "other" },
  };
  assert.deepEqual(
    mergePages([
      page([order("a")], "next"),
      page([order("a"), other, order("b")]),
    ]).data.map((o) => [o.book.venue_id, o.id]),
    [
      ["test", "a"],
      ["other", "a"],
      ["test", "b"],
    ],
  );
});
test("one live connection appends pages once and reconciles all loaded data", async (t) => {
  const Socket = sockets(t);
  let state: FeedState | undefined;
  let calls = 0;
  let newer = false;
  t.mock.method(
    globalThis,
    "fetch",
    async (_url: string, options: RequestInit) => {
      assert.equal(JSON.parse(options.body as string).consistency, "live");
      calls++;
      return new Response(JSON.stringify(page([order(newer ? "d" : "c")])));
    },
  );
  const feed = new OrderFeed(query, false, false, (value) => {
    state = value;
  });
  t.after(() => feed.close());
  feed.start();
  Socket.opened[0].deliver(page([order("a"), order("b")], "next"));
  await until(() => !!state && !state.loadingMore);
  feed.loadMore();
  feed.loadMore();
  await until(() => state!.page.data.length === 3 && !state!.loadingMore);
  assert.equal(calls, 1);
  assert.equal(Socket.opened.length, 1);
  newer = true;
  Socket.opened[0].deliver(page([order("x"), order("a")], "new-boundary"));
  await until(() => state!.page.data.at(-1)?.id === "d");
  assert.deepEqual(
    state!.page.data.map((o) => o.id),
    ["x", "a", "d"],
  );
  assert.equal(calls, 2);
});
test("closing a feed prevents slow continuations leaking across currency changes", async (t) => {
  const Socket = sockets(t);
  let state: FeedState | undefined;
  let finish: (response: Response) => void = () => {};
  t.mock.method(
    globalThis,
    "fetch",
    async () =>
      await new Promise<Response>((resolve) => {
        finish = resolve;
      }),
  );
  const first = new OrderFeed(query, false, false, (value) => {
    state = value;
  });
  first.start();
  Socket.opened[0].deliver(page([order("a")], "next"));
  await until(() => !!state && !state.loadingMore);
  first.loadMore();
  first.close();
  const second = new OrderFeed(
    { ...query, scope: { ...query.scope, currency_id: "XTR" } },
    false,
    false,
    (value) => {
      state = value;
    },
  );
  t.after(() => second.close());
  second.start();
  Socket.opened[1].deliver(page([order("stars", "XTR")]));
  await until(() => state!.page.data[0]?.id === "stars");
  finish(new Response(JSON.stringify(page([order("old")]))));
  await tick();
  await tick();
  assert.equal(state!.page.data[0].book.currency_id, "XTR");
  assert.equal(state!.page.data.length, 1);
});
test("expired snapshots restart the loaded window and repeated continuations stop", async (t) => {
  sockets(t);
  let state: FeedState | undefined;
  const tokens: string[] = [];
  let expired = true;
  t.mock.method(
    globalThis,
    "fetch",
    async (_url: string, options: RequestInit) => {
      const token = JSON.parse(options.body as string).page_token;
      tokens.push(token);
      if (token && expired) {
        expired = false;
        return new Response(
          JSON.stringify({
            error: { code: "restart_read", message: "Expired" },
          }),
          { status: 409 },
        );
      }
      return new Response(
        JSON.stringify(
          token
            ? page([order("c")], "repeated")
            : page([order("a"), order("b")], "repeated"),
        ),
      );
    },
  );
  const feed = new OrderFeed(query, false, true, (value) => {
    state = value;
  });
  t.after(() => feed.close());
  feed.start();
  await until(
    () => !!state && !state.loadingMore && state.page.data.length === 2,
  );
  feed.loadMore();
  await until(() => state!.page.data.length === 3 && !state!.loadingMore);
  assert.deepEqual(tokens, ["", "repeated", "", "repeated"]);
  feed.loadMore();
  await until(() => !!state!.error);
  assert.match(state!.error, /repeated continuation/);
  assert.equal(state!.loadingMore, false);
});

test("history omits active-only consistency and paused reads force snapshots", async (t) => {
  for (const history of [false, true]) {
    let state: FeedState | undefined;
    t.mock.method(
      globalThis,
      "fetch",
      async (url: string, options: RequestInit) => {
        const body = JSON.parse(options.body as string);
        assert.equal(url, history ? "/executions/query" : "/orders/query");
        assert.equal(body.consistency, history ? undefined : "snapshot");
        return new Response(JSON.stringify(page([order("only")])));
      },
    );
    const feed = new OrderFeed(
      { ...query, consistency: "live" },
      history,
      !history,
      (value) => {
        state = value;
      },
    );
    feed.start();
    await until(() => state?.page.data.length === 1);
    feed.close();
  }
});
