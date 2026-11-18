import assert from "node:assert/strict";
import { test } from "node:test";
import { StrategyClient, guideKey } from "../src/valuation/strategies.ts";
import { ResourceCache } from "../src/resources/cache.ts";
import type { ValuationStrategy } from "../src/market.ts";
const signal = () => new AbortController().signal;
const row = (id: string, revision = "r1"): ValuationStrategy => ({
  id,
  name: id,
  description: "description",
  documentation_url: `/guides/${id}`,
  documentation_revision: revision,
});

test("strategy metadata shares requests, survives one cancelled consumer and expires on use", async () => {
  let calls = 0,
    now = 0,
    aborted = false;
  let release!: (rows: ValuationStrategy[]) => void;
  const client = new StrategyClient(
    async (stop) => {
      calls++;
      stop?.addEventListener("abort", () => {
        aborted = true;
      });
      return await new Promise<ValuationStrategy[]>((resolve) => {
        release = resolve;
      });
    },
    () => now,
  );
  const stop = new AbortController();
  const first = client.get("one", stop.signal),
    second = client.get("one", signal());
  stop.abort();
  await assert.rejects(first);
  assert.equal(aborted, false);
  release([row("one")]);
  assert.equal((await second).id, "one");
  assert.equal((await client.get("one", signal())).id, "one");
  assert.equal(calls, 1);
  now = 60_000;
  const refreshed = client.get("one", signal());
  release([row("one", "r2")]);
  assert.equal((await refreshed).documentation_revision, "r2");
  assert.equal(calls, 2);
});

test("new result IDs revalidate cached metadata once and unknown IDs do not create retry storms", async () => {
  let calls = 0;
  const client = new StrategyClient(async () =>
    ++calls === 1 ? [row("old")] : [row("old"), row("new")],
  );
  await client.get("old", signal());
  assert.equal((await client.get("new", signal())).id, "new");
  assert.equal(calls, 2);
  await assert.rejects(client.get("unknown", signal()));
  assert.equal(calls, 3);
  await assert.rejects(client.get("unknown", signal()));
  assert.equal(calls, 3);
  client.invalidate();
  await assert.rejects(client.get("unknown", signal()));
  assert.equal(calls, 4);
});

test("fresh unknown IDs do not immediately repeat a network read and failures remain retryable", async () => {
  let calls = 0;
  const client = new StrategyClient(async () => {
    if (++calls === 1) throw Error("offline");
    return [row("one")];
  });
  await assert.rejects(client.get("one", signal()));
  assert.equal((await client.get("one", signal())).id, "one");
  assert.equal(calls, 2);
  client.invalidate();
  await assert.rejects(client.get("unknown", signal()));
  assert.equal(calls, 3);
});

test("resource TTL and invalidation share retention, and guide keys change with their revision", async () => {
  let now = 0,
    calls = 0;
  const cache = new ResourceCache(
    async () => ({ value: ++calls, bytes: 1 }),
    8,
    1,
    100,
    () => now,
  );
  assert.equal(await cache.get("key", signal()), 1);
  now = 99;
  assert.equal(await cache.get("key", signal()), 1);
  now = 100;
  assert.equal(cache.hasFresh("key"), false);
  assert.equal(await cache.get("key", signal()), 2);
  cache.invalidate("key");
  assert.equal(await cache.get("key", signal()), 3);
  assert.notEqual(guideKey(row("one", "r1")), guideKey(row("one", "r2")));
});
