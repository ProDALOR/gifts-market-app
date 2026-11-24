// Shared wire fixtures for the browser suites; no live hub or Telegram dependency.
export function criteriaReport(params) {
  return {
    target: {
      ...params.target,
      criteria: {
        ...params.target.criteria,
        number: params.target.criteria.number ?? undefined,
      },
    },
    currency_id: params.currency_id,
    evaluated_at: new Date().toISOString(),
    stale: false,
    results: [
      {
        strategy: "ask_floor",
        revision: "r1",
        data: { value: "12", count: 3 },
      },
    ],
  };
}
const calculatedAt = new Map();
export function floorReport(
  params,
  models = [],
  backdrops = [],
  patterns = [],
) {
  const floor = (value, count = 3) => ({
    value,
    count,
  });
  return {
    target: params.target,
    currency_id: params.currency_id,
    stale: false,
    evaluated_at: new Date().toISOString(),
    results: [
      {
        strategy: "catalog_floor",
        revision: "r1",
        data: {
          collections: params.target.collection_ids.map((collection_id) => {
            const key = params.currency_id + ":" + collection_id;
            if (!calculatedAt.has(key))
              calculatedAt.set(key, new Date().toISOString());
            const details = params.target.details;
            return {
              collection_id,
              evaluated_at: calculatedAt.get(key),
              floor: floor("8", 7),
              ...(details?.collection_id === collection_id
                ? {
                    traits: {
                      models: Object.fromEntries(
                        models.map((id) => [id, floor("10.000000001", 2)]),
                      ),
                      backdrops: Object.fromEntries(
                        backdrops.map((id) => [id, floor("9", 1)]),
                      ),
                      patterns: Object.fromEntries(
                        patterns.map((id) => [id, floor("8.5", 3)]),
                      ),
                      ...(details.pair
                        ? {
                            pair: {
                              ...details.pair,
                              floor: floor("12.5", 1),
                            },
                          }
                        : {}),
                    },
                  }
                : {}),
            };
          }),
        },
      },
    ],
  };
}
export function giftReport(params, data) {
  const {
    strategy,
    stale,
    evaluated_at,
    gift,
    slug,
    currency_id,
    revision = "r1",
    ...metrics
  } = data;
  return {
    target:
      params.target.kind === "gift_attributes"
        ? { ...params.target, number: gift?.number ?? params.target.number }
        : { kind: "gift", gift, ...(slug ? { slug } : {}) },
    currency_id,
    evaluated_at,
    stale,
    results: [{ strategy, revision, data: metrics }],
  };
}

/** Static discovery definitions shared by fixtures; values only occur in reports. */
export function fixtureDisplay(id) {
  const currency = (id, label, decimals = null, primary = false) => ({
    id,
    label,
    primary,
    format: { kind: "currency", decimals },
  });
  if (id === "best_bid")
    return {
      fields: [currency("best", "Best bid", 2, true)],
      evidence: [],
      note: "Observed bid before fees.",
    };
  return id === "gift_floor"
    ? {
        fields: [
          currency("final_mark", "Final mark", 2, true),
          currency("collection_floor", "Collection floor"),
          currency("model_floor", "Model floor"),
          currency("backdrop_floor", "Backdrop floor"),
          currency("model_backdrop_floor", "Model & backdrop floor"),
          {
            id: "model_multiplier",
            label: "Model multiplier",
            primary: false,
            format: { kind: "number", decimals: 2, suffix: "×" },
          },
          {
            id: "backdrop_multiplier",
            label: "Backdrop multiplier",
            primary: false,
            format: { kind: "number", decimals: 2, suffix: "×" },
          },
        ],
        evidence: [{ path: "/collection_floor/count", label: "gifts" }],
        note: "Current asks, before fees.",
      }
    : {
        fields: [
          { ...currency("price", "Matching ask floor", null, true), path: "" },
        ],
        evidence: [{ path: "/count", label: "gifts" }],
        note: "Current matching asks, before fees.",
      };
}

export function bestBidResult(value = "1.5") {
  const quote = {
    value,
    order: { id: "bid-1", venue_id: "venue", quantity: 3 },
  };
  return {
    strategy: "best_bid",
    revision: "r1",
    data: {
      best: quote,
      references: {
        collection: quote,
        model: quote,
        backdrop: quote,
        model_backdrop: quote,
        pattern: quote,
      },
    },
  };
}
