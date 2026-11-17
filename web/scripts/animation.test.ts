import assert from "node:assert/strict";
import { test } from "node:test";
import { gzipSync, strToU8 } from "fflate";
import { decodeTgs, loadAnimationData } from "../src/animation-data.ts";
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
