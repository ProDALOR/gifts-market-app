import assert from "node:assert/strict";
import { test } from "node:test";
import { setImmediate as flush } from "node:timers/promises";
import {
  MediaResource,
  MediaFailure,
  MediaHttpError,
  retryMedia,
} from "../src/previews/media-resource.ts";
import { loadAnimationData } from "../src/previews/animation-data.ts";
import { loadPattern } from "../src/previews/pattern-data.ts";
import { QueueFullError } from "../src/resources/task-queue.ts";
import { fetchMedia } from "../src/previews/media-client.ts";
import { gzipSync } from "node:zlib";
const signal = () => new AbortController().signal;

test("a disconnect during body transfer stays retryable instead of poisoning the asset as invalid", async (t) => {
  let now = 0,
    calls = 0;
  t.mock.method(Date, "now", () => now);
  t.mock.method(
    globalThis,
    "fetch",
    async () =>
      new Response(
        ++calls === 1
          ? new ReadableStream({
              start(controller) {
                controller.error(new TypeError("connection reset"));
              },
            })
          : new Uint8Array([1]),
        { headers: { "Content-Type": "image/png" } },
      ),
  );
  const resource = new MediaResource(async (url, stop) => ({
    value: await fetchMedia(url, stop, "image"),
    bytes: 1,
  }));
  await assert.rejects(
    resource.get("/disconnect", signal()),
    (error) =>
      error instanceof MediaFailure &&
      error.retryable &&
      error.retryAt === 2000,
  );
  await assert.rejects(resource.get("/disconnect", signal()));
  assert.equal(calls, 1);
  now = 2000;
  assert.deepEqual(
    await resource.get("/disconnect", signal()),
    new Uint8Array([1]),
  );
  assert.equal(calls, 2);
});

test("invalid animation data retries after expiry, then shares the recovered decoded value", async (t) => {
  let now = 0,
    calls = 0;
  t.mock.method(Date, "now", () => now);
  const data = {
    v: "5.7.4",
    w: 512,
    h: 512,
    fr: 60,
    ip: 0,
    op: 180,
    layers: [],
    assets: [],
  };
  t.mock.method(globalThis, "fetch", async () => {
    calls++;
    return new Response(
      now === 0 ? "invalid TGS" : gzipSync(JSON.stringify(data)),
      { headers: { "Content-Type": "application/x-tgsticker" } },
    );
  });
  for (let i = 0; i < 3; i++)
    await assert.rejects(
      loadAnimationData("/invalid-then-valid", signal()),
      MediaFailure,
    );
  assert.equal(calls, 1);
  now = 60_000;
  const first = await loadAnimationData("/invalid-then-valid", signal());
  assert.deepEqual(first, data);
  assert.equal(await loadAnimationData("/invalid-then-valid", signal()), first);
  assert.equal(calls, 2);
});

test("missing animation and pattern requests share negative caching across visits and colors", async (t) => {
  let now = 0,
    calls = 0;
  t.mock.method(Date, "now", () => now);
  t.mock.method(globalThis, "fetch", async () => {
    calls++;
    return new Response(null, { status: 404 });
  });
  for (let i = 0; i < 3; i++) {
    await assert.rejects(
      loadAnimationData("/negative-animation", signal()),
      MediaFailure,
    );
    await assert.rejects(
      loadPattern("/negative-pattern", i % 2 ? "#112233" : "#223344", signal()),
      MediaFailure,
    );
  }
  assert.equal(calls, 2);
  now = 60_000;
  await assert.rejects(loadAnimationData("/negative-animation", signal()));
  await assert.rejects(loadPattern("/negative-pattern", "#112233", signal()));
  assert.equal(calls, 4);
});

test("decoding failures have bounded retention; successful resources retain their original TTL", async (t) => {
  let now = 0,
    calls = 0;
  t.mock.method(Date, "now", () => now);
  const resource = new MediaResource(async (key) => {
    calls++;
    if (key === "bad") throw Error("invalid data");
    return { value: key, bytes: 1 };
  });
  await assert.rejects(
    resource.get("bad", signal()),
    (error) => error instanceof MediaFailure && !error.retryable,
  );
  await resource.get("good", signal());
  now = 59_999;
  assert.equal(resource.hasFresh("bad"), true);
  await assert.rejects(resource.get("bad", signal()));
  now = 60_000;
  await assert.rejects(resource.get("bad", signal()));
  await resource.get("good", signal());
  assert.equal(calls, 3);
});

test("shared retry deadlines respect Retry-After and one cancelled waiter leaves the other retrying", async (t) => {
  let now = 0,
    calls = 0;
  t.mock.timers.enable({ apis: ["setTimeout"] });
  t.mock.method(Date, "now", () => now);
  const resource = new MediaResource(async () => {
    if (++calls === 1) throw new MediaHttpError(503, 60_000);
    return { value: true, bytes: 1 };
  });
  const stop = new AbortController();
  const a = retryMedia(() => resource.get("key", stop.signal), stop.signal);
  const other = signal();
  const b = retryMedia(() => resource.get("key", other), other);
  await flush();
  assert.equal(calls, 1);
  stop.abort();
  await assert.rejects(a);
  now = 59_999;
  t.mock.timers.tick(59_999);
  await flush();
  assert.equal(calls, 1);
  now = 60_000;
  t.mock.timers.tick(1);
  assert.equal(await b, true);
  assert.equal(calls, 2);
});

test("failure retention is bounded and queue pressure is not recorded as a missing asset", async () => {
  const calls = new Map<string, number>();
  const resource = new MediaResource(async (key) => {
    calls.set(key, (calls.get(key) ?? 0) + 1);
    throw key === "queue" ? new QueueFullError() : new MediaHttpError(404);
  }, 8);
  for (let i = 0; i < 12; i++)
    await assert.rejects(resource.get(String(i), signal()));
  await assert.rejects(resource.get("0", signal()));
  assert.equal(
    calls.get("0"),
    2,
    "Old failures are evicted by the byte budget",
  );
  for (let i = 0; i < 2; i++)
    await assert.rejects(resource.get("queue", signal()), QueueFullError);
  assert.equal(calls.get("queue"), 2);
});

test("queue pressure uses cancellable retry waits without caching a missing asset", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let now = 0,
    calls = 0;
  t.mock.method(Date, "now", () => now);
  const resource = new MediaResource(async () => {
    calls++;
    throw new QueueFullError();
  });
  const stop = new AbortController();
  const result = retryMedia(
    () => resource.get("queue", stop.signal),
    stop.signal,
  );
  await flush();
  assert.equal(calls, 1);
  assert.equal(resource.hasFresh("queue"), false);
  now = 150;
  t.mock.timers.tick(150);
  await flush();
  assert.equal(calls, 2);
  stop.abort();
  await assert.rejects(result);
  now = 1000;
  t.mock.timers.tick(1000);
  await flush();
  assert.equal(calls, 2);
});

test("producer timeouts retry as transient failures while consumer cancellation stays uncached", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let now = 0,
    calls = 0;
  t.mock.method(Date, "now", () => now);
  const resource = new MediaResource(async (key, stop) => {
    if (++calls === 1 || key === "cancel")
      await new Promise((_, reject) =>
        stop.addEventListener("abort", () => reject(stop.reason), {
          once: true,
        }),
      );
    return { value: true, bytes: 1 };
  });
  const active = signal();
  const result = retryMedia(() => resource.get("timeout", active), active);
  await flush();
  now = 35_000;
  t.mock.timers.tick(35_000);
  await flush();
  assert.equal(calls, 1);
  assert.equal(resource.hasFresh("timeout"), true);
  now = 37_000;
  t.mock.timers.tick(2000);
  assert.equal(await result, true);
  assert.equal(calls, 2);
  const stop = new AbortController();
  const cancelled = resource.get("cancel", stop.signal);
  await flush();
  stop.abort();
  await assert.rejects(cancelled);
  assert.equal(resource.hasFresh("cancel"), false);
});

test("small image transfers fill a bounded eight-slot budget while scroll previews warm", async (t) => {
  const releases: (() => void)[] = [];
  let active = 0,
    peak = 0;
  t.mock.method(
    globalThis,
    "fetch",
    () =>
      new Promise<Response>((resolve) => {
        peak = Math.max(peak, ++active);
        releases.push(() => {
          active--;
          resolve(
            new Response(new Uint8Array([1]), {
              headers: { "Content-Type": "image/png" },
            }),
          );
        });
      }),
  );
  const reads = Array.from({ length: 12 }, (_, i) =>
    fetchMedia(`/bounded-image-${i}`, signal(), "image"),
  );
  await flush();
  assert.equal(releases.length, 8);
  assert.equal(peak, 8);
  while (releases.length) {
    releases.splice(0).forEach((release) => release());
    await flush();
  }
  await Promise.all(reads);
  assert.equal(active, 0);
  assert.equal(peak, 8);
});
