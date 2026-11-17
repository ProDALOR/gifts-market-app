import assert from "node:assert/strict";
import { test } from "node:test";
import { MediaStore, fetchMedia, retryDelay } from "../src/media-client.ts";
const signal = () => new AbortController().signal;
test("resource cache uses byte limits and least-recently-used eviction", async () => {
  const calls: string[] = [];
  const store = new MediaStore(async (key) => {
    calls.push(key);
    return { value: key, bytes: 4 };
  }, 8);
  await store.get("a", signal());
  await store.get("b", signal());
  await store.get("a", signal());
  await store.get("c", signal());
  await store.get("a", signal());
  await store.get("b", signal());
  assert.deepEqual(calls, ["a", "b", "c", "b"]);
});
test("pending resources are bounded, shared, and cancelled by their last consumer", async () => {
  let aborted = false;
  const store = new MediaStore(
    async (_key, signal) => {
      await new Promise((_, reject) =>
        signal.addEventListener("abort", () => {
          aborted = true;
          reject(signal.reason);
        }),
      );
      return { value: "never", bytes: 1 };
    },
    8,
    1,
  );
  const firstStop = new AbortController(),
    secondStop = new AbortController();
  const first = store.get("same", firstStop.signal),
    second = store.get("same", secondStop.signal);
  await assert.rejects(store.get("other", signal()), /Too many/);
  firstStop.abort();
  await assert.rejects(first);
  assert.equal(aborted, false);
  secondStop.abort();
  await assert.rejects(second);
  assert.equal(aborted, true);
});
test("failed resources are removed and oversized values are not retained", async () => {
  let calls = 0;
  const store = new MediaStore(async () => {
    if (++calls === 1) throw Error("unavailable");
    return { value: "large", bytes: 20 };
  }, 8);
  await assert.rejects(store.get("key", signal()));
  assert.equal(await store.get("key", signal()), "large");
  await store.get("key", signal());
  assert.equal(calls, 3);
});
test("retry policy honors bounded Retry-After and never retries a missing asset", () => {
  const response = (status: number, after?: string) =>
    new Response(null, {
      status,
      headers: after ? { "Retry-After": after } : {},
    });
  assert.equal(retryDelay(response(503, "3")), 3000);
  assert.equal(retryDelay(response(429)), 2000);
  assert.equal(retryDelay(response(404)), undefined);
  assert.equal(retryDelay(response(503, "300")), undefined);
  assert.equal(retryDelay(response(503, "invalid")), undefined);
});
test("downloads reject invalid types, declared oversize and chunked oversize", async (t) => {
  for (const response of [
    new Response("<svg>", { headers: { "Content-Type": "image/svg+xml" } }),
    new Response("small", {
      headers: { "Content-Length": String(256 * 1024 + 1) },
    }),
    new Response(new Uint8Array(256 * 1024 + 1)),
  ]) {
    t.mock.method(globalThis, "fetch", async () => response);
    await assert.rejects(fetchMedia("/media", signal(), "image"));
  }
});
test("a transient response retries once and abort cancels backoff", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let calls = 0;
  t.mock.method(globalThis, "fetch", async () =>
    ++calls === 1
      ? new Response(null, { status: 503, headers: { "Retry-After": "1" } })
      : new Response("ok", { headers: { "Content-Type": "image/png" } }),
  );
  const result = fetchMedia("/media", signal(), "image");
  await Promise.resolve();
  await Promise.resolve();
  t.mock.timers.tick(1000);
  assert.equal(new TextDecoder().decode(await result), "ok");
  assert.equal(calls, 2);
  t.mock.method(
    globalThis,
    "fetch",
    async () => new Response(null, { status: 503 }),
  );
  const stop = new AbortController();
  const cancelled = fetchMedia("/media", stop.signal, "image");
  await Promise.resolve();
  await Promise.resolve();
  stop.abort();
  await assert.rejects(cancelled);
});
