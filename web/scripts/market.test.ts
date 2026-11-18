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
      { target: { kind: "unsupported", reason: "unsupported" } },
      options,
    ),
    undefined,
  );
});

test("valuation requests preserve strategy data and structured failure evidence", async (t) => {
  const { valuation, ApiFailure } = await import("../src/market.ts");
  const query = {
    strategy: "model_multiplier" as const,
    collection_id: "gift",
    model_id: "model",
    currency_id: "GRAM",
  };
  const mark = {
    strategy: "model_multiplier",
    data: { factor: "3.333333333333" },
    revision: "r1",
    evaluated_at: "2026-11-18T12:00:00Z",
  };
  const details = {
    strategy: "model_multiplier",
    required_samples: 3,
    available_samples: 1,
    context: { kind: "multiplier", trait_samples: 1 },
  };
  let fail = false;
  t.mock.method(globalThis, "fetch", async (url, init) => {
    assert.equal(url, "/valuations/query");
    assert.equal(init.method, "POST");
    assert.deepEqual(JSON.parse(init.body), query);
    return fail
      ? Response.json(
          {
            error: {
              code: "not_enough_data",
              message: "Missing evidence",
              details,
            },
          },
          { status: 422 },
        )
      : Response.json(mark);
  });
  assert.deepEqual(await valuation(query), mark);
  fail = true;
  await assert.rejects(valuation(query), (error: unknown) => {
    assert.ok(error instanceof ApiFailure);
    assert.equal(error.code, "not_enough_data");
    assert.deepEqual(error.details, details);
    return true;
  });
});
