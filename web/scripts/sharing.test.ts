import assert from "node:assert/strict";
import { ApiFailure } from "../src/api-error.ts";
import { test } from "node:test";
import { SharingController } from "../src/sharing.ts";
import type { SharedMessage, ShareTarget } from "../src/sharing.ts";
import type { WebApp } from "../src/telegram.ts";
import type { SessionRequest } from "../src/telegram-session.ts";
const target: ShareTarget = {
  type: "gift",
  slug: "Gift-123",
  currency_id: "GRAM",
};
const report = (): SharedMessage => ({
  id: "prepared",
  expires_at: Math.floor(Date.now() / 1000) + 600,
  inline_query: "s:0123456789abcdef",
});
function setup(
  options: { modern?: boolean; prepared?: boolean; throws?: boolean } = {},
) {
  const inline: string[] = [],
    native: string[] = [];
  let callback: ((sent: boolean) => void) | undefined;
  const app = {
    isVersionAtLeast: () => options.modern !== false,
    switchInlineQuery(query: string) {
      inline.push(query);
    },
    shareMessage(id: string, cb: (sent: boolean) => void) {
      if (options.throws) throw new Error("unsupported");
      native.push(id);
      callback = cb;
    },
  } as unknown as WebApp;
  const request = (async () =>
    options.prepared === false
      ? { ...report(), id: undefined }
      : report()) as SessionRequest;
  const value = new SharingController(app, request);
  return {
    value,
    app,
    request,
    inline,
    native,
    finish: (sent: boolean) => callback!(sent),
  };
}
test("prepared sharing uses native chooser; cancelling never opens inline fallback", async () => {
  const { value, native, inline, finish } = setup();
  await value.share(target);
  assert.deepEqual(native, ["prepared"]);
  assert.equal(value.current().busy, true);
  await value.share(target);
  assert.equal(native.length, 1);
  finish(false);
  assert.deepEqual(inline, []);
  assert.equal(value.current().busy, false);
});
test("old clients, unavailable prepared IDs and synchronous SDK failures use exact inline fallback", async () => {
  for (const option of [
    { modern: false },
    { prepared: false },
    { throws: true },
  ]) {
    const { value, inline } = setup(option);
    await value.share(target);
    assert.deepEqual(inline, [report().inline_query]);
    assert.equal(value.current().busy, false);
  }
});
test("all target types forward unchanged; late responses after switching pages cannot open a chooser", async () => {
  const { app, native, inline } = setup();
  let resolve!: (value: SharedMessage) => void;
  let signal: AbortSignal | undefined;
  let sent: unknown;
  const value = new SharingController(app, ((path, body, cancel) => {
    assert.equal(path, "/telegram/share");
    sent = body;
    signal = cancel;
    return new Promise((r) => {
      resolve = r as typeof resolve;
    });
  }) as SessionRequest);
  const portfolio = {
    type: "portfolio",
    query: {
      user_id: 7,
      currency_id: "GRAM",
      sort: "position_desc",
      offset: 30,
      limit: 30,
      version: "a".repeat(32),
      filters: {
        collection_id: "collection",
        models: ["model"],
        backdrops: [],
        patterns: [],
      },
    },
  } as ShareTarget;
  const pending = value.share(portfolio);
  assert.deepEqual(sent, portfolio);
  value.cancel();
  assert.equal(signal?.aborted, true);
  resolve(report());
  await pending;
  assert.deepEqual(native, []);
  assert.deepEqual(inline, []);
  assert.deepEqual(value.current(), { busy: false, error: "" });
});
test("expired reports and preparation failures show an error without sending a different selection", async () => {
  for (const request of [
    async () => ({ ...report(), expires_at: 1 }),
    async () => {
      throw new Error("Portfolio changed; reload");
    },
  ]) {
    const { app, inline, native } = setup();
    const value = new SharingController(app, request as SessionRequest);
    await value.share(target);
    assert.ok(value.current().error);
    assert.equal(value.current().busy, false);
    assert.deepEqual(native, []);
    assert.deepEqual(inline, []);
  }
});
test("the preparation deadline aborts work and ignores responses that arrive afterwards", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 1800000000000 });
  const { app, native } = setup();
  let resolve!: (value: SharedMessage) => void;
  const value = new SharingController(
    app,
    (() =>
      new Promise((r) => {
        resolve = r as typeof resolve;
      })) as SessionRequest,
  );
  const pending = value.share(target);
  t.mock.timers.tick(28000);
  assert.match(value.current().error, /too long/);
  resolve(report());
  await pending;
  assert.deepEqual(native, []);
});

test("gift inline sharing remains available without prepared support or a signed-in API session", async () => {
  for (const code of ["unauthorized", "unavailable", "not_found"]) {
    const { app, inline, native } = setup();
    const value = new SharingController(app, (async () => {
      throw new ApiFailure(code, "not configured");
    }) as SessionRequest);
    await value.share(target);
    assert.deepEqual(inline, ["Gift-123 GRAM"]);
    assert.deepEqual(native, []);
    assert.deepEqual(value.current(), { busy: false, error: "" });
  }
});

test("a broken inline SDK reports an error instead of rejecting an unobserved click handler", async () => {
  const { app } = setup({ modern: false });
  app.switchInlineQuery = () => {
    throw new TypeError("unsupported");
  };
  const value = new SharingController(app, (async () =>
    report()) as SessionRequest);
  await value.share(target);
  assert.match(value.current().error, /update Telegram/);
  assert.equal(value.current().busy, false);
});
