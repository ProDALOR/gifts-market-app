import assert from "node:assert/strict";
import { test } from "node:test";
import { StrategyClient, guideKey } from "../src/valuation/strategies.ts";
import { ResourceCache } from "../src/resources/cache.ts";
import type { ValuationStrategy } from "../src/market.ts";
const signal = () => new AbortController().signal;
const row = (id: string, revision = "r1"): ValuationStrategy => ({
  purpose: "estimate",
  id,
  name: id,
  description: "description",
  input: { targets: ["selection"], period: "unsupported" },
  documentation_url: `/guides/${id}`,
  documentation_revision: revision,
  revision,
  display: { fields: [], evidence: [], note: "" },
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
  const first = client.list(stop.signal),
    second = client.list(signal());
  stop.abort();
  await assert.rejects(first);
  assert.equal(aborted, false);
  release([row("one")]);
  assert.equal((await second)[0].id, "one");
  assert.equal((await client.list(signal()))[0].id, "one");
  assert.equal(calls, 1);
  now = 60_000;
  const refreshed = client.list(signal());
  release([row("one", "r2")]);
  assert.equal((await refreshed)[0].documentation_revision, "r2");
  assert.equal(calls, 2);
});

test("failed discovery stays retryable and explicit invalidation reloads the catalog", async () => {
  let calls = 0;
  const client = new StrategyClient(async () => {
    if (++calls === 1) throw Error("offline");
    return [row("one")];
  });
  await assert.rejects(client.list(signal()));
  assert.equal((await client.list(signal()))[0].id, "one");
  assert.equal(calls, 2);
  client.invalidate();
  await client.list(signal());
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

test("strategy compatibility uses discovered capabilities rather than strategy IDs", async () => {
  const { compatibleStrategies } =
    await import("../src/valuation/strategies.ts");
  const rows: ValuationStrategy[] = [
    {
      ...row("renamed_floor"),
      purpose: "reference",
      input: { targets: ["gift", "selection"], period: "unsupported" },
    },
    {
      ...row("renamed_composite"),
      input: { targets: ["gift"], period: "unsupported" },
    },
    {
      ...row("renamed_history"),
      input: { targets: ["gift", "selection"], period: "required" },
    },
  ];
  assert.deepEqual(
    compatibleStrategies(rows, "gift").map((row) => row.id),
    ["renamed_floor", "renamed_composite"],
  );
  assert.deepEqual(
    compatibleStrategies(rows, "selection").map((row) => row.id),
    ["renamed_floor"],
  );
  assert.deepEqual(
    compatibleStrategies(rows, "selection", true).map((row) => row.id),
    ["renamed_floor", "renamed_history"],
  );
  assert.deepEqual(compatibleStrategies(rows, "catalog", true), []);
  assert.deepEqual(
    compatibleStrategies(rows, "gift", false, "estimate").map((row) => row.id),
    ["renamed_composite"],
    "Gift previews select estimates without hardcoding strategy IDs",
  );
  assert.deepEqual(
    compatibleStrategies(rows, "selection", false).map((row) => row.id),
    ["renamed_floor"],
    "Bids retain matching reference floors",
  );
});
