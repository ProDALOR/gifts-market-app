import assert from "node:assert/strict";
import { test } from "node:test";
import { CollectionCatalogCache } from "../src/catalog/collection-cache.ts";
import { collectionOptions } from "../src/catalog/selectors.ts";
import type { Collection, CollectionCatalog } from "../src/market.ts";
const collection = (
  id: string,
  ids: string[],
): Collection & { models: string[] } => ({
  id,
  name: id,
  preview: null,
  models: ids,
});
const signal = () => new AbortController().signal;
const result = (
  gift: Collection & { models: string[] },
): CollectionCatalog => ({
  collection: gift,
  models: (gift.models || []).map((id) => ({ id, name: id, media: null })),
  backdrops: [],
  patterns: [],
  cursor: "test",
  sync: { ready: true, state: "live", updated_at: null },
});
test("collection visits reuse complete responses and catalog changes invalidate them", async () => {
  let calls = 0;
  const definitions = new Map([
    ["gift", collection("gift", ["a", "b"])],
    ["other", collection("other", ["c"])],
  ]);
  const cache = new CollectionCatalogCache(async (id) => {
    calls++;
    return result(definitions.get(id)!);
  });
  const first = definitions.get("gift")!,
    second = definitions.get("other")!;
  await cache.load(first, signal());
  await cache.load(second, signal());
  await cache.load({ ...first }, signal());
  assert.equal(calls, 2);
  assert.deepEqual(
    collectionOptions(first, "models", cache.options).map((row) => row.id),
    ["a", "b"],
  );
  const changed = { ...collection("gift", ["a", "b", "d"]), name: "Changed" };
  definitions.set("gift", changed);
  await cache.load(changed, signal());
  assert.equal(calls, 3);
});
test("failed or cancelled collection responses do not commit and remain retryable", async () => {
  let calls = 0;
  const gift = collection("gift", ["a", "b"]);
  const cache = new CollectionCatalogCache(async () => {
    if (++calls === 1) throw Error("temporary");
    return result(gift);
  });
  await assert.rejects(cache.load(gift, signal()), /temporary/);
  assert.equal(cache.options.size, 0);
  await cache.load(gift, signal());
  assert.equal(cache.options.size, 2);
  const stop = new AbortController();
  const abandoned = new CollectionCatalogCache(async () => {
    stop.abort();
    return result(gift);
  });
  await assert.rejects(abandoned.load(gift, stop.signal));
  assert.equal(abandoned.options.size, 0);
});
test("one typed collection request loads large model catalogs without generic fan-out", async () => {
  let calls = 0;
  const gift = collection(
    "gift",
    Array.from({ length: 700 }, (_, i) => String(i)),
  );
  const cache = new CollectionCatalogCache(async (id) => {
    calls++;
    assert.equal(id, gift.id);
    return result(gift);
  });
  await cache.load(gift, signal());
  assert.equal(calls, 1);
  assert.equal(cache.options.size, 700);
});

test("only the active collection exposes options while other visits stay cached", async () => {
  let models = ["old"];
  const cache = new CollectionCatalogCache(async (id) =>
    result(collection(id, id === "gift" ? models : ["other-model"])),
  );
  await cache.load(collection("gift", ["old"]), signal());
  await cache.load(collection("other", ["other-model"]), signal());
  models = ["new"];
  await cache.load(
    { ...collection("gift", ["new"]), name: "Updated" },
    signal(),
  );
  assert.deepEqual([...cache.options.keys()].sort(), ["new"]);
});

test("newer collection responses win when cached collections share an option", async () => {
  let name = "Old";
  const cache = new CollectionCatalogCache(async (id) => ({
    ...result(collection(id, ["shared"])),
    models: [{ id: "shared", name, media: null }],
  }));
  const first = collection("first", ["shared"]);
  await cache.load(first, signal());
  await cache.load(collection("second", ["shared"]), signal());
  name = "Updated";
  await cache.load({ ...first, name: "New catalog definition" }, signal());
  assert.equal(cache.options.get("shared")?.name, "Updated");
});

test("collection cache evicts least recent entries and bounds retained choices", async () => {
  const calls: string[] = [];
  const cache = new CollectionCatalogCache(async (id) => {
    calls.push(id);
    return result(collection(id, [id]));
  });
  for (let i = 0; i < 9; i++)
    await cache.load(collection(String(i), [String(i)]), signal());
  await cache.load(collection("1", ["1"]), signal());
  assert.equal(calls.length, 9);
  assert.deepEqual([...cache.options.keys()], ["1"]);
  await cache.load(collection("0", ["0"]), signal());
  assert.equal(calls.length, 10);
  const huge = collection(
    "huge",
    Array.from({ length: 10001 }, (_, i) => `m${i}`),
  );
  const large = new CollectionCatalogCache(async () => result(huge));
  await large.load(huge, signal());
  await large.load(huge, signal()); // oversized response is usable but not retained
  assert.equal(large.options.size, 10001);
});
