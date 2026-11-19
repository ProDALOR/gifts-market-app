import assert from "node:assert/strict";
import { test } from "node:test";
import { setImmediate as flush } from "node:timers/promises";
import { loadOutline } from "../src/previews/outline-resource.ts";

const signal = () => new AbortController().signal;

test("missing outlines are shared across consumers and remounts, then expire", async (t) => {
  let now = 0,
    calls = 0;
  t.mock.method(Date, "now", () => now);
  t.mock.method(globalThis, "fetch", async () => {
    calls++;
    return new Response(null, { status: 404 });
  });
  const url = "/outlines/missing";
  assert.deepEqual(
    await Promise.all([loadOutline(url, signal()), loadOutline(url, signal())]),
    [null, null],
  );
  now = 59_999;
  assert.equal(await loadOutline(url, signal()), null);
  assert.equal(calls, 1);
  now = 60_000;
  await loadOutline(url, signal());
  assert.equal(calls, 2);
});

test("transient misses respect Retry-After and bound negative cache lifetime", async (t) => {
  let now = 0,
    calls = 0;
  t.mock.method(Date, "now", () => now);
  t.mock.method(globalThis, "fetch", async () => {
    calls++;
    return new Response(null, {
      status: 503,
      headers: { "Retry-After": "20" },
    });
  });
  const url = "/outlines/transient";
  await loadOutline(url, signal());
  now = 19_999;
  await loadOutline(url, signal());
  assert.equal(calls, 1);
  now = 20_000;
  await loadOutline(url, signal());
  assert.equal(calls, 2);
  t.mock.method(globalThis, "fetch", async () => {
    calls++;
    return new Response(null, {
      status: 429,
      headers: { "Retry-After": "999999" },
    });
  });
  await loadOutline("/outlines/long-retry", signal());
  now += 300_000;
  await loadOutline("/outlines/long-retry", signal());
  assert.equal(calls, 4);
});

test("network failures have short negative caching", async (t) => {
  let now = 0,
    calls = 0;
  t.mock.method(Date, "now", () => now);
  t.mock.method(globalThis, "fetch", async () => {
    calls++;
    throw Error("offline");
  });
  const url = "/outlines/offline";
  await loadOutline(url, signal());
  now = 1999;
  await loadOutline(url, signal());
  assert.equal(calls, 1);
  now = 2000;
  await loadOutline(url, signal());
  assert.equal(calls, 2);
});

test("one consumer can leave; last-consumer cancellation is never a cached miss", async (t) => {
  let calls = 0,
    aborts = 0;
  t.mock.method(globalThis, "fetch", (_url, options) => {
    calls++;
    return new Promise<Response>((_resolve, reject) => {
      options!.signal!.addEventListener(
        "abort",
        () => {
          aborts++;
          reject(options!.signal!.reason);
        },
        { once: true },
      );
    });
  });
  const first = new AbortController(),
    second = new AbortController();
  const a = loadOutline("/outlines/cancel", first.signal);
  const b = loadOutline("/outlines/cancel", second.signal);
  await flush();
  first.abort();
  await assert.rejects(a);
  assert.equal(aborts, 0);
  second.abort();
  await assert.rejects(b);
  await flush();
  const third = new AbortController();
  const c = loadOutline("/outlines/cancel", third.signal);
  await flush();
  assert.equal(calls, 2);
  third.abort();
  await assert.rejects(c);
  assert.equal(aborts, 2);
});
