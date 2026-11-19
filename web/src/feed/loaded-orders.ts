import type { components } from "../gen/api.ts";
import { OrderCoverage, type OrderRange } from "./coverage.ts";
import { orderKey } from "../orderbook/order-activity.ts";
type Result = components["schemas"]["OrderSnapshotResponse"];
type Entry = Result["data"][number];
export function lowerBound(rows: readonly Entry[], key: string) {
  let lo = 0,
    hi = rows.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (rows[mid].key < key) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}
/** A persistent identity index and sorted visited rows. Only covered rows are reconciled. */
export class LoadedOrders {
  rows: Entry[] = [];
  private identities = new Map<string, Entry>();
  private coverage = new OrderCoverage();
  get hasBefore() {
    return this.coverage.hasBefore;
  }
  get unread() {
    return this.coverage.gap();
  }
  unreadWithin(range: OrderRange) {
    return this.coverage.gap(range);
  }
  coveredRange(key: string) {
    return this.coverage.containing(key);
  }
  merge(view: Result, requested?: OrderRange) {
    const { start, end } = view.range;
    // A limited baseline certifies only its returned coverage. Retain rows
    // beyond it, but reopen the interval before the next cached row.
    if (requested && end !== null && end !== requested.end) {
      let join = this.rows[lowerBound(this.rows, end)]?.key ?? requested.end;
      if (requested.end !== null && (join === null || join > requested.end))
        join = requested.end;
      if (join === null || end < join)
        this.coverage.remove({ start: end, end: join });
    }
    const first = start === null ? 0 : lowerBound(this.rows, start);
    const last = end === null ? this.rows.length : lowerBound(this.rows, end);
    const moved = new Set<Entry>();
    const incoming = view.data.map((row) => {
      const old = this.identities.get(orderKey(row));
      if (old && (old.key < (start ?? "") || (end !== null && old.key >= end)))
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
    this.coverage.add(view.range);
  }
}
