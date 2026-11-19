import { performance } from "node:perf_hooks";
import { LoadedOrders } from "../src/feed/loaded-orders.ts";
const row = (id) => ({
  id: String(id),
  book: { venue_id: "v", collection_id: "g", currency_id: "GRAM" },
  kind: "ask",
  price: "1",
  quantity: 1,
  observed_at: "2026-11-18T12:00:00Z",
  target: {
    kind: "criteria",
    models: [],
    backdrops: [],
    patterns: [],
    number: null,
  },
  // Keep baseline payload fields in this fixture for comparable object sizes.
  position: `p.${id}`,
  sort_key: String(id).padStart(16, "0"),
  key: String(id).padStart(16, "0"),
});
const bound = (row) => row.key;
for (const count of [10_000, 100_000]) {
  const rows = Array.from({ length: count }, (_, i) => row(i));
  const cache = new LoadedOrders();
  cache.merge({ data: rows, range: { start: null, end: null } });
  const start = Math.floor(count / 2) - 100;
  const data = rows.slice(start, start + 200);
  const samples = [];
  for (let i = 0; i < 72; i++) {
    const next = data.map((row, j) =>
      j === 100 ? { ...row, price: String(i + 2) } : row,
    );
    const before = performance.now();
    cache.merge({
      data: next,
      range: { start: bound(data[0]), end: bound(data.at(-1)) },
    });
    if (i >= 8) samples.push(performance.now() - before);
  }
  samples.sort((a, b) => a - b);
  console.log(
    JSON.stringify({
      case: "cached_number_sorted_price_update",
      cached: count,
      active: 200,
      median_ms: samples[samples.length / 2],
    }),
  );
}
