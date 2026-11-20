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
export function marketValues(params, models = [], backdrops = []) {
  const floor = (value) => ({
    price: { value },
    evidence: { listings: 3, gifts: 3 },
  });
  return {
    currency_id: params.currency_id,
    evaluated_at: new Date().toISOString(),
    stale: false,
    collections: params.collection_ids.map((collection_id) => ({
      collection_id,
      floor: floor("8"),
    })),
    ...(params.traits_for
      ? {
          traits: {
            collection_id: params.traits_for,
            models: Object.fromEntries(
              models.map((id) => [id, floor("10.000000001")]),
            ),
            backdrops: Object.fromEntries(
              backdrops.map((id) => [id, floor("9")]),
            ),
            ...(params.pair
              ? { pair: { ...params.pair, floor: floor("12.5") } }
              : {}),
          },
        }
      : {}),
  };
}
