import assert from "node:assert/strict";
import { test } from "node:test";
import { amount, modelMedia } from "../src/market.ts";
import type { Order, CatalogOption } from "../src/market.ts";
test("amount formatting preserves precision beyond JavaScript numeric range", () => {
  assert.equal(
    amount("10000000000000000.000000001"),
    "10,000,000,000,000,000.000000001",
  );
  assert.equal(amount("0.500000001"), "0.500000001");
});
test("concrete previews use supplied media, never guessed paths or hypothetical targets", () => {
  const gift: Pick<Order, "target"> = {
    target: {
      kind: "gift",
      id: "42",
      number: 42,
      traits: { model: "model", backdrop: null, pattern: null },
    },
  };
  const media = {
    thumbnail_url: "/assets/thumbnail?v=2",
    animation_url: "/assets/animation.tgs?v=3",
  };
  const options = new Map<string, CatalogOption>([
    ["model", { id: "model", name: "Model", kind: "model", media }],
  ]);
  assert.equal(modelMedia(gift, options), media);
  assert.equal(modelMedia(gift, new Map()), undefined);
  const missing = new Map<string, CatalogOption>([
    ["model", { id: "model", name: "Model", kind: "model", media: null }],
  ]);
  assert.equal(modelMedia(gift, missing), undefined);
  assert.equal(
    modelMedia(
      {
        target: {
          kind: "criteria",
          models: ["model"],
          backdrops: [],
          patterns: [],
          number: null,
        },
      },
      options,
    ),
    undefined,
  );
  assert.equal(
    modelMedia(
      {
        target: {
          kind: "unsupported",
          code: "unsupported_constraints",
          message: "Unsupported conditions",
        },
      },
      options,
    ),
    undefined,
  );
});

test("gift valuation preserves every strategy outcome and uses typed requests and preserves structured failures", async (t) => {
  const { valuation, ApiFailure } = await import("../src/market.ts");
  const query = { gift: "https://t.me/nft/Gift-123", currency_id: "GRAM" };
  const metric = { value: "6" };
  const report: import("../src/market.ts").GiftValuationResponse = {
    strategy: "gift_floor",
    revision: "r1",
    evaluated_at: "2026-11-18T12:00:00Z",
    slug: "Gift-123",
    gift: {
      id: "gift123",
      collection_id: "gift",
      number: 123,
      traits: { model: "model", backdrop: null, pattern: null },
    },
    currency_id: "GRAM",
    sync: { ready: true, state: "live", updated_at: null },
    collection_floor: metric,
    model_floor: metric,
    backdrop_floor: metric,
    model_backdrop_floor: metric,
    model_multiplier: metric,
    backdrop_multiplier: metric,
    final_mark: metric,
    evidence: {
      collection: { listings: 3, gifts: 3 },
      model: { listings: 3, gifts: 3 },
      backdrop: { listings: 3, gifts: 3 },
      model_backdrop: { listings: 1, gifts: 1 },
      minimum_multiplier_gifts: 3,
    },
    display: {
      fields: [
        {
          id: "strategy_value",
          label: "A strategy-defined value",
          primary: true,
          value: { kind: "currency", amount: "6.00", currency_id: "GRAM" },
        },
        {
          id: "strategy_issue",
          label: "An unavailable value",
          primary: false,
          value: {
            kind: "unavailable",
            label: "Unavailable",
            message: "Missing observations",
            detail: "1 of 3",
          },
        },
      ],
      summary: "Strategy summary",
      note: "Strategy note",
    },
  };
  let fail = false;
  const { liveSocket } = await import("../src/live-socket.ts");
  t.mock.method(liveSocket, "request", async (call) => {
    assert.deepEqual(call, { method: "valuation.evaluate", params: query });
    if (fail) throw new ApiFailure("not_found", "Gift is not in market data");
    return report;
  });
  assert.deepEqual(await valuation(query), report);
  fail = true;
  await assert.rejects(valuation(query), (error: unknown) => {
    assert.ok(error instanceof ApiFailure);
    assert.equal(error.code, "not_found");
    return true;
  });
});

test("catalog reads preserve opaque compact versions from strong and weak ETags", async (t) => {
  const { catalog, collectionCatalog } = await import("../src/market.ts");
  const token = "f3MgDfNxr-4kEnhwdAqItQ";
  let tag: string | undefined;
  const paths: string[] = [];
  t.mock.method(globalThis, "fetch", async (path: string) => {
    paths.push(path);
    return new Response("{}", { headers: tag ? { etag: tag } : {} });
  });
  for (const [header, expected] of [
    [`"${token}"`, token],
    [`W/"${token}"`, token],
    ['"catalog-v2-legacy"', "catalog-v2-legacy"],
    [undefined, undefined],
  ]) {
    tag = header;
    for (const read of [
      catalog,
      (signal: AbortSignal, received?: (value: string) => void) =>
        collectionCatalog("gift/id", signal, received),
    ]) {
      let received: string | undefined;
      await read(new AbortController().signal, (value) => {
        received = value;
      });
      assert.equal(received, expected);
    }
  }
  assert.deepEqual(
    paths,
    Array.from({ length: 4 }, () => [
      "/catalog",
      "/collections/gift%2Fid",
    ]).flat(),
  );
});
