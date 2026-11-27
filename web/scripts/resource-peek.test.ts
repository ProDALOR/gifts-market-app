import { test } from "node:test";
import assert from "node:assert/strict";
import { ResourceCache } from "../src/resources/cache.ts";
test("decoded peeks return only fresh successes and retain their LRU position", async () => {
  let now = 0,
    calls = 0;
  const cache = new ResourceCache(
    async (key) => {
      calls++;
      return { value: { key }, bytes: 1 };
    },
    2,
    1,
    10,
    () => now,
  );
  const signal = new AbortController().signal;
  const first = await cache.get("a", signal);
  await cache.get("b", signal);
  assert.equal(cache.peek("a"), first);
  await cache.get("c", signal);
  assert.equal(cache.peek("b"), undefined);
  assert.equal(cache.peek("a"), first);
  assert.equal(calls, 3);
  now = 10;
  assert.equal(cache.peek("a"), undefined);
});
test("pending work and cached failures cannot be treated as decoded media", async () => {
  let release!: (value: { value: string; bytes: number }) => void;
  const cache = new ResourceCache(
    () =>
      new Promise((resolve) => {
        release = resolve;
      }),
  );
  const result = cache.get("a", new AbortController().signal);
  assert.equal(cache.peek("a"), undefined);
  release({ value: "ready", bytes: 1 });
  await result;
  assert.equal(cache.peek("a"), "ready");
  const failed = new ResourceCache(
    async () => {
      throw Error("missing");
    },
    10,
    1,
    100,
    () => 0,
    (error) => ({ error, expires: 10 }),
  );
  await assert.rejects(failed.get("bad", new AbortController().signal));
  assert.equal(failed.hasFresh("bad"), true);
  assert.equal(failed.peek("bad"), undefined);
});
