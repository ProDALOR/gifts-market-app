import assert from "node:assert/strict";
import { test } from "node:test";
import { gzipSync, strToU8 } from "fflate";
import {
  decodeTgs,
  loadAnimationData,
} from "../src/previews/animation-data.ts";
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
const tgs = () => gzipSync(strToU8(JSON.stringify(data)));
test("TGS decoding validates compressed data, dimensions and expanded size", () => {
  assert.deepEqual(decodeTgs(tgs()), data);
  assert.throws(() => decodeTgs(new Uint8Array([1, 2, 3])));
  assert.throws(() => decodeTgs(gzipSync(new Uint8Array(2 * 1024 * 1024 + 1))));
  assert.throws(() =>
    decodeTgs(gzipSync(strToU8(JSON.stringify({ ...data, w: 1000000 })))),
  );
});
test("row/detail previews share a download and one cancelled preview leaves the other running", async (t) => {
  let calls = 0;
  let finish: (() => void) | undefined;
  let providerSignal: AbortSignal | undefined;
  t.mock.method(
    globalThis,
    "fetch",
    async (_url: string, options: RequestInit) => {
      calls++;
      providerSignal = options.signal!;
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      return new Response(tgs());
    },
  );
  const firstStop = new AbortController(),
    secondStop = new AbortController();
  const first = loadAnimationData("/animation-shared", firstStop.signal);
  const second = loadAnimationData("/animation-shared", secondStop.signal);
  firstStop.abort();
  await assert.rejects(first);
  assert.equal(providerSignal!.aborted, false);
  finish!();
  assert.deepEqual(await second, data);
  assert.deepEqual(
    await loadAnimationData("/animation-shared", new AbortController().signal),
    data,
  );
  assert.equal(calls, 1);
});
test("leaving the last preview cancels its network request and a later preview can retry", async (t) => {
  let providerSignal: AbortSignal | undefined;
  t.mock.method(
    globalThis,
    "fetch",
    async (_url: string, options: RequestInit) => {
      providerSignal = options.signal!;
      await new Promise((_, reject) =>
        providerSignal!.addEventListener(
          "abort",
          () => reject(providerSignal!.reason),
          { once: true },
        ),
      );
      return new Response(tgs());
    },
  );
  const stop = new AbortController();
  const pending = loadAnimationData("/animation-cancelled", stop.signal);
  stop.abort();
  await assert.rejects(pending);
  assert.equal(providerSignal!.aborted, true);
  t.mock.method(globalThis, "fetch", async () => new Response(tgs()));
  assert.deepEqual(
    await loadAnimationData(
      "/animation-cancelled",
      new AbortController().signal,
    ),
    data,
  );
});

test("renderer selection preserves independent gradient opacity without changing source data", async () => {
  const { animationRenderer } =
    await import("../src/previews/animation-data.ts");
  const colors = [0, 1, 1, 1, 0.8, 1, 1, 1, 1, 1, 1, 1];
  const aligned = [...colors, 0, 1, 0.8, 0.5, 1, 0];
  const separate = [...colors, 0, 1, 0.5, 0.5, 1, 0];
  const scene = (values: unknown, kind = "gf") => ({
    ...data,
    layers: [
      {
        ty: 4,
        shapes: [{ ty: "gr", it: [{ ty: kind, g: { p: 3, k: values } }] }],
      },
    ],
  });
  assert.equal(animationRenderer(data), "canvas");
  assert.equal(animationRenderer(scene({ a: 0, k: colors })), "canvas");
  assert.equal(animationRenderer(scene({ a: 0, k: aligned })), "canvas");
  const independent = scene({ a: 0, k: separate });
  const original = JSON.stringify(independent);
  assert.equal(animationRenderer(independent), "svg");
  assert.equal(
    animationRenderer(independent),
    "svg",
    "Shared data reuse the renderer decision",
  );
  assert.equal(
    JSON.stringify(independent),
    original,
    "Telegram shapes and colors remain untouched",
  );
  assert.equal(
    animationRenderer(scene({ a: 0, k: [...colors, 0, 1, 1, 0] }, "gs")),
    "svg",
  );
  assert.equal(
    animationRenderer(
      scene({
        a: 1,
        k: [
          { t: 0, s: aligned, e: separate },
          { t: 90, s: separate },
        ],
      }),
    ),
    "svg",
  );
  assert.equal(
    animationRenderer({
      ...data,
      assets: [{ id: "nested", layers: independent.layers }],
    }),
    "svg",
  );
});
