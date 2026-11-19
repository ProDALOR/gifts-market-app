import assert from "node:assert/strict";
import { test } from "node:test";
import { setImmediate as flush } from "node:timers/promises";
import { ResourceCache } from "../src/resources/cache.ts";
import { fetchMedia } from "../src/previews/media-client.ts";
import {
  MediaHttpError,
  MediaResource,
  retryMedia,
} from "../src/previews/media-resource.ts";
const signal = () => new AbortController().signal;
test("resource cache uses byte limits and least-recently-used eviction", async () => {
  const calls: string[] = [];
  const store = new ResourceCache(async (key) => {
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
  const store = new ResourceCache(
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
  const queuedStop = new AbortController();
  const queued = store.get("other", queuedStop.signal);
  queuedStop.abort();
  await assert.rejects(queued);
  firstStop.abort();
  await assert.rejects(first);
  assert.equal(aborted, false);
  secondStop.abort();
  await assert.rejects(second);
  assert.equal(aborted, true);
});
test("failed resources are removed and oversized values are not retained", async () => {
  let calls = 0;
  const store = new ResourceCache(async () => {
    if (++calls === 1) throw Error("unavailable");
    return { value: "large", bytes: 20 };
  }, 8);
  await assert.rejects(store.get("key", signal()));
  assert.equal(await store.get("key", signal()), "large");
  await store.get("key", signal());
  assert.equal(calls, 3);
});
test("downloads report one attempt and the full Retry-After to the shared retry policy", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  t.mock.method(Date, "now", () => 0);
  for (const [status, after, delay] of [
    [503, "3", 3000],
    [429, undefined, 2000],
    [503, new Date(3000).toUTCString(), 3000],
    [404, undefined, undefined],
    [503, "300", 300_000],
    [500, "60", 60_000],
    [404, "60", 60_000],
    [503, "invalid", undefined],
  ] as const) {
    let calls = 0;
    t.mock.method(globalThis, "fetch", async () => {
      calls++;
      return new Response(null, {
        status,
        headers: after ? { "Retry-After": after } : {},
      });
    });
    const result = fetchMedia("/media", signal(), "image").catch(
      (error) => error,
    );
    await flush();
    const error = await result;
    assert.ok(error instanceof MediaHttpError);
    assert.equal(error.retryAfterMs, delay);
    assert.equal(calls, 1);
  }
});
test("downloads reject invalid types, declared oversize and chunked oversize", async (t) => {
  for (const response of [
    new Response("<html>", { headers: { "Content-Type": "text/html" } }),
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
  let calls = 0,
    now = 0;
  t.mock.method(Date, "now", () => now);
  const resource = new MediaResource(async (url, stop) => ({
    value: await fetchMedia(url, stop, "image"),
    bytes: 1,
  }));
  t.mock.method(globalThis, "fetch", async () =>
    ++calls === 1
      ? new Response(null, { status: 503, headers: { "Retry-After": "1" } })
      : new Response("ok", { headers: { "Content-Type": "image/png" } }),
  );
  const active = signal();
  const result = retryMedia(() => resource.get("/media", active), active);
  await flush();
  now = 1000;
  t.mock.timers.tick(1000);
  assert.equal(new TextDecoder().decode(await result), "ok");
  assert.equal(calls, 2);
  t.mock.method(
    globalThis,
    "fetch",
    async () => new Response(null, { status: 503 }),
  );
  const stop = new AbortController();
  const cancelled = retryMedia(
    () => resource.get("/cancel", stop.signal),
    stop.signal,
  );
  await flush();
  stop.abort();
  await assert.rejects(cancelled);
});

test("background retry delays release transfer slots for interactive media and cancel cleanly", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let now = 0;
  t.mock.method(Date, "now", () => now);
  const resource = new MediaResource(async (url, stop, priority) => ({
    value: await fetchMedia(url, stop, "image", priority),
    bytes: 1,
  }));
  const calls = new Map<string, number>();
  t.mock.method(globalThis, "fetch", async (input) => {
    const url = String(input);
    const count = (calls.get(url) ?? 0) + 1;
    calls.set(url, count);
    return url.startsWith("/background/") && count === 1
      ? new Response(null, { status: 503, headers: { "Retry-After": "1" } })
      : new Response("ok", {
          headers: {
            "Content-Type":
              url === "/interactive"
                ? "application/x-tgsticker"
                : "image/svg+xml",
          },
        });
  });
  const stops = Array.from({ length: 4 }, () => new AbortController());
  const background = stops.map((stop, i) =>
    retryMedia(
      () => resource.get(`/background/${i}`, stop.signal, -10),
      stop.signal,
    ),
  );
  await flush();
  assert.equal(calls.size, 4);
  const interactive = await fetchMedia("/interactive", signal(), "tgs", 100);
  assert.equal(
    new TextDecoder().decode(interactive),
    "ok",
    "Interactive transfer runs before backoff elapses",
  );
  const cancelled = assert.rejects(background[0]);
  stops[0].abort();
  await cancelled;
  now = 1000;
  t.mock.timers.tick(1000);
  await Promise.all(background.slice(1));
  assert.equal(calls.get("/background/0"), 1);
  assert.equal(calls.get("/background/1"), 2);
});

test("SVG recovery accepts the preview budget and keeps other image limits", async (t) => {
  t.mock.method(
    globalThis,
    "fetch",
    async () =>
      new Response(new Uint8Array(512 * 1024), {
        headers: { "Content-Type": "image/svg+xml" },
      }),
  );
  assert.equal(
    (await fetchMedia("/preview", signal(), "image")).length,
    512 * 1024,
  );
  t.mock.method(
    globalThis,
    "fetch",
    async () =>
      new Response(new Uint8Array(1024 * 1024 + 1), {
        headers: { "Content-Type": "image/svg+xml" },
      }),
  );
  await assert.rejects(fetchMedia("/preview", signal(), "image"));
});

test("bundled TGS downloads accept static-file MIME without accepting unrelated types", async (t) => {
  for (const type of ["application/x-tgsticker", "application/octet-stream"]) {
    t.mock.method(
      globalThis,
      "fetch",
      async () =>
        new Response(new Uint8Array([1, 2]), {
          headers: { "Content-Type": type },
        }),
    );
    assert.equal(
      (await fetchMedia("/assets/brand.tgs", signal(), "tgs")).length,
      2,
    );
  }
  t.mock.method(
    globalThis,
    "fetch",
    async () =>
      new Response("html", { headers: { "Content-Type": "text/html" } }),
  );
  await assert.rejects(fetchMedia("/assets/brand.tgs", signal(), "tgs"));
});
