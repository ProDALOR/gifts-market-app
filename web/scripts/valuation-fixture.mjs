// Shared wire fixtures for the browser suites; no live hub or Telegram dependency.
export function criteriaReport(params) {
  return {
    target: { ...params.target },
    currency_id: params.currency_id,
    evaluated_at: new Date().toISOString(),
    stale: false,
    results: [
      {
        strategy: "ask_floor",
        data: { price: { value: "12" }, evidence: { listings: 3, gifts: 3 } },
        display: {
          fields: [
            {
              id: "ask_floor",
              label: "Matching ask floor",
              primary: true,
              value: {
                kind: "currency",
                amount: "12",
                currency_id: params.currency_id,
              },
            },
          ],
          note: "Current matching asks, before fees.",
        },
      },
    ],
  };
}
const calculatedAt = new Map();
export function floorReport(params, models = [], backdrops = []) {
  const floor = (value) => ({
    price: { value },
    evidence: { listings: 3, gifts: 3 },
  });
  return {
    currency_id: params.currency_id,
    stale: false,
    collections: params.collection_ids.map((collection_id) => {
      const key = params.currency_id + ":" + collection_id;
      if (!calculatedAt.has(key))
        calculatedAt.set(key, new Date().toISOString());
      const details = params.details;
      return {
        collection_id,
        evaluated_at: calculatedAt.get(key),
        floor: floor("8"),
        ...(details?.collection_id === collection_id
          ? {
              traits: {
                models: Object.fromEntries(
                  models.map((id) => [id, floor("10.000000001")]),
                ),
                backdrops: Object.fromEntries(
                  backdrops.map((id) => [id, floor("9")]),
                ),
                ...(details.pair
                  ? { pair: { ...details.pair, floor: floor("12.5") } }
                  : {}),
              },
            }
          : {}),
      };
    }),
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
    display,
    ...metrics
  } = data;
  return {
    target: { kind: "gift", gift, ...(slug ? { slug } : {}) },
    currency_id,
    evaluated_at,
    stale,
    results: [{ strategy, data: metrics, display }],
  };
}
