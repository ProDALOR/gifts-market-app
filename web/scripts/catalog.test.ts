import assert from "node:assert/strict";
import { test } from "node:test";
import { CatalogOptionsCache } from "../src/catalog/options-cache.ts";
import { collectionOptions } from "../src/catalog/selectors.ts";
import type { Collection, CatalogOption } from "../src/orderbook.ts";
const collection = (id: string, ids: string[]) =>
  ({ id, attributes: { "tg.model": { allowed_values: ids } } }) as Collection;
const option = (id: string) =>
  ({
    kind: "attribute_option",
    id,
    name: id,
    attribute_id: "tg.model",
  }) as CatalogOption;
const signal = () => new AbortController().signal;
test("catalog visits reuse completed loads and metadata changes invalidate them", async () => {
  let calls = 0;
  const cache = new CatalogOptionsCache(async (_kind, _signal, ids) => {
    calls++;
    return ids!.map(option);
  });
  const first = collection("gift", ["a", "b"]),
    second = collection("other", ["c"]);
  await cache.load(first, signal());
  await cache.load(second, signal());
  await cache.load({ ...first }, signal());
  assert.equal(calls, 2);
  assert.deepEqual(
    collectionOptions(first, "tg.model", cache.options).map((row) => row.id),
    ["a", "b"],
  );
  await cache.load(collection("gift", ["a", "b", "d"]), signal());
  assert.equal(calls, 3);
});
test("failed or aborted catalog batches never commit incomplete choices and can retry", async () => {
  const stop = new AbortController();
  let attempt = 0;
  const gift = collection(
    "gift",
    Array.from({ length: 501 }, (_, i) => String(i)),
  );
  const cache = new CatalogOptionsCache(async (_kind, _signal, ids) => {
    if (++attempt === 2) throw Error("temporary failure");
    return ids!.map(option);
  });
  await assert.rejects(cache.load(gift, signal()), /temporary/);
  assert.equal(cache.options.size, 0);
  await cache.load(gift, signal());
  assert.equal(cache.options.size, 501);
  const abandoned = new CatalogOptionsCache(async () => {
    stop.abort();
    return [option("x")];
  });
  await assert.rejects(
    abandoned.load(collection("abandoned", ["x"]), stop.signal),
  );
  assert.equal(abandoned.options.size, 0);
});
test("numeric facts never trigger a full option scan, and choice batches match API pages", async () => {
  const calls: string[][] = [];
  const gift = collection(
    "gift",
    Array.from({ length: 297 }, (_, i) => String(i)),
  );
  gift.attributes["tg.number"] = { required: false };
  const cache = new CatalogOptionsCache(async (_kind, _signal, ids) => {
    assert.ok(ids?.length, "Only explicitly requested choice IDs are loaded");
    calls.push(ids);
    return ids.map(option);
  });
  await cache.load(gift, signal());
  assert.deepEqual(
    calls.map((ids) => ids.length),
    [200, 97],
  );
  assert.equal(cache.options.size, 297);
});
