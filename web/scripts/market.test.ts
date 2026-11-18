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
      gift: {
        id: "42",
        collection_id: "gift",
        number: 42,
        traits: { model: "model", backdrop: null, pattern: null },
      },
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
          criteria: {
            models: ["model"],
            backdrops: [],
            patterns: [],
            number: null,
          },
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
          reason: {
            code: "unsupported_constraints",
            message: "Unsupported conditions",
          },
        },
      },
      options,
    ),
    undefined,
  );
});

test("gift valuation preserves every strategy outcome and only throws for HTTP failures", async (t) => {
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
      collection_samples: 3,
      model_samples: 3,
      backdrop_samples: 3,
      model_backdrop_samples: 1,
      minimum_multiplier_samples: 3,
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
  t.mock.method(globalThis, "fetch", async (url, init) => {
    assert.equal(url, "/valuations/gift");
    assert.equal(init.method, "POST");
    assert.deepEqual(JSON.parse(init.body), query);
    return fail
      ? Response.json(
          {
            error: { code: "not_found", message: "Gift is not in market data" },
          },
          { status: 404 },
        )
      : Response.json(report);
  });
  assert.deepEqual(await valuation(query), report);
  fail = true;
  await assert.rejects(valuation(query), (error: unknown) => {
    assert.ok(error instanceof ApiFailure);
    assert.equal(error.code, "not_found");
    return true;
  });
});
