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
  prepared: { id: "prepared", expires_at: Math.floor(Date.now() / 1000) + 600 },
  fallback: { type: "inline", query: "Gift-123 GRAM" },
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
      ? { ...report(), prepared: undefined }
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
test("old clients, unavailable prepared IDs and synchronous SDK failures use readable inline fallback", async () => {
  for (const option of [
    { modern: false },
    { prepared: false },
    { throws: true },
  ]) {
    const { value, inline } = setup(option);
    await value.share(target);
    assert.deepEqual(inline, ["Gift-123 GRAM"]);
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
    target: {
      user_id: 7,
      currency_id: "GRAM",
      sort: "position_desc",
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
test("expired native IDs use the timeless target fallback", async () => {
  const { app, native, inline } = setup();
  const value = new SharingController(app, (async () => ({
    ...report(),
    prepared: { id: "old", expires_at: 1 },
  })) as SessionRequest);
  await value.share(target);
  assert.deepEqual(native, []);
  assert.deepEqual(inline, ["Gift-123 GRAM"]);
  assert.equal(value.current().error, "");
});
test("non-recoverable preparation failures show their error", async () => {
  const { app, inline } = setup();
  const value = new SharingController(app, (async () => {
    throw Error("Invalid target");
  }) as SessionRequest);
  await value.share(target);
  assert.equal(value.current().error, "Invalid target");
  assert.deepEqual(inline, []);
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

test("large target links use Telegram's share chooser without a snapshot token", async () => {
  const { app, inline, native } = setup();
  const opened: string[] = [];
  app.openTelegramLink = (url) => opened.push(url);
  const url =
    "https://app.example/#f=c%3Dgift%7Cm%3Dmodel-1%2Cmodel-2&currency=GRAM";
  const controller = new SharingController(app, (async () => ({
    fallback: { type: "link", url },
  })) as SessionRequest);
  await controller.share(target);
  assert.deepEqual(inline, []);
  assert.deepEqual(native, []);
  assert.equal(new URL(opened[0]).searchParams.get("url"), url);
  assert.equal(controller.current().error, "");
});
test("portfolio targets preserve filters and sort when the prepared API is unavailable", async () => {
  const { app } = setup();
  const opened: string[] = [];
  app.openTelegramLink = (url) => opened.push(url);
  const controller = new SharingController(
    app,
    (async () => {
      throw new ApiFailure("unauthorized", "Sign in");
    }) as SessionRequest,
    "https://app.example/",
  );
  await controller.share({
    type: "portfolio",
    target: {
      user_id: 7,
      currency_id: "XTR",
      sort: "number_desc",
      filters: {
        collection_id: "gift",
        models: ["model"],
        backdrops: ["black"],
        patterns: ["sun"],
        number: 42,
      },
    },
  });
  const url = new URL(new URL(opened[0]).searchParams.get("url")!);
  const params = new URLSearchParams(url.hash.slice(1));
  assert.equal(url.pathname, "/portfolio/7");
  assert.equal(params.get("sort"), "number_desc");
  assert.equal(params.get("f"), "c=gift|m=model|b=black|s=sun|n=42");
  assert.equal(params.has("version"), false);
  assert.equal(params.has("offset"), false);
  assert.equal(controller.current().error, "");
});
test("oversized inline targets are never truncated or sent", async () => {
  const { app, inline } = setup();
  const controller = new SharingController(app, (async () => ({
    fallback: { type: "inline", query: "x".repeat(257) },
  })) as SessionRequest);
  await controller.share(target);
  assert.deepEqual(inline, []);
  assert.ok(controller.current().error);
});
