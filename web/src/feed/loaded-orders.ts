import type { components } from "../gen/api.ts";
import { orderKey } from "../orderbook/order-activity.ts";
type Result = components["schemas"]["OrderSnapshotResponse"];
type Entry = Result["data"][number];
export function lowerBound(
  rows: readonly Entry[],
  key: string,
  exclusive = false,
) {
  let lo = 0,
    hi = rows.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (rows[mid].key < key || (exclusive && rows[mid].key === key))
      lo = mid + 1;
    else hi = mid;
  }
  return lo;
}
/** A persistent identity index and sorted visited rows. Only covered rows are reconciled. */
export class LoadedOrders {
  rows: Entry[] = [];
  private identities = new Map<string, Entry>();
  hasBefore = false;
  hasAfter = true;
  merge(view: Result) {
    const { start, end } = view.range;
    const first = start === null ? 0 : lowerBound(this.rows, start);
    const last =
      end === null ? this.rows.length : lowerBound(this.rows, end, true);
    const moved = new Set<Entry>();
    const incoming = view.data.map((row) => {
      const old = this.identities.get(orderKey(row));
      if (old && (old.key < (start ?? "") || (end !== null && old.key > end)))
        moved.add(old);
      return old && (old === row || JSON.stringify(old) === JSON.stringify(row))
        ? old
        : row;
    });
    for (let i = first; i < last; i++)
      this.identities.delete(orderKey(this.rows[i]));
    for (const row of incoming) this.identities.set(orderKey(row), row);
    const head = this.rows.slice(0, first),
      tail = this.rows.slice(last);
    this.rows = (
      moved.size ? head.filter((row) => !moved.has(row)) : head
    ).concat(
      incoming,
      moved.size ? tail.filter((row) => !moved.has(row)) : tail,
    );
    if (start === null || !this.rows.length || this.rows[0].key >= start)
      this.hasBefore = start !== null;
    if (end === null || !this.rows.length || this.rows.at(-1)!.key <= end)
      this.hasAfter = end !== null;
  }
}
