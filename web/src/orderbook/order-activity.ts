import type { Order, Page } from "../market.ts";
export const FRESHNESS_MS = 60_000;
const MAX_REMOVED = 50;
export const orderKey = (order: Order) =>
  JSON.stringify([order.book.venue_id, order.id]);
export type OrderActivity = { state: "new" | "removed"; since: number };
type Removed = { order: Order; index: number };
/** Local view transitions; leaving a limited page's tail is not evidence of removal. */
export class OrderActivityTracker {
  private previous: Page<Order>[] = [];
  private transitions = new Map<string, OrderActivity>();
  private removed = new Map<string, Removed>();
  private removedPositions: number[] = [];
  private baseline = true;
  rebase() {
    this.baseline = true;
  }
  get needsBaseline() {
    return this.baseline;
  }
  update(
    pages: Page<Order>[],
    orders: Order[],
    refreshed: ReadonlySet<number>,
    now = Date.now(),
  ) {
    this.expire(now);
    const present = new Set(orders.map(orderKey));
    const before = this.previous.flatMap((page) => page.data);
    const known = new Set(before.map(orderKey));
    if (this.baseline) {
      this.transitions.clear();
      this.removed.clear();
    } else {
      // Pagination never marks its newly downloaded tail as new. Only live head arrivals do.
      for (const order of pages[0]?.data || []) {
        const key = orderKey(order);
        if (!known.has(key) || this.transitions.get(key)?.state === "removed")
          this.transitions.set(key, { state: "new", since: now });
      }
      const candidates = new Set<string>();
      for (const index of refreshed) {
        const start = this.previous
          .slice(0, index)
          .reduce((count, page) => count + page.data.length, 0);
        const current = pages[index];
        if (!current) continue;
        const currentKeys = new Set(current.data.map(orderKey));
        // A surviving anchor confirms coverage. Rows pushed beyond it can still be active.
        let anchor = -1;
        for (let i = before.length - 1; i >= 0; i--) {
          if (currentKeys.has(orderKey(before[i]))) {
            anchor = i;
            break;
          }
        }
        const end = current.next_page_token ? anchor : before.length - 1;
        for (const order of before.slice(start, end + 1))
          candidates.add(orderKey(order));
        // A complete head covers the whole loaded query, including pruned continuation pages.
        if (index === 0 && !current.next_page_token)
          for (const order of before) candidates.add(orderKey(order));
      }
      before.forEach((order, index) => {
        const key = orderKey(order);
        if (
          !present.has(key) &&
          candidates.has(key) &&
          !this.removed.has(key)
        ) {
          this.removed.set(key, { order, index });
          this.transitions.set(key, { state: "removed", since: now });
        }
      });
    }
    for (const key of present) {
      this.removed.delete(key);
      if (this.transitions.get(key)?.state === "removed")
        this.transitions.delete(key);
    }
    while (this.removed.size > MAX_REMOVED) {
      const key = this.removed.keys().next().value!;
      this.removed.delete(key);
      this.transitions.delete(key);
    }
    for (const key of this.transitions.keys()) {
      if (!present.has(key) && !this.removed.has(key))
        this.transitions.delete(key);
    }
    this.previous = pages.slice();
    this.baseline = false;
    return this.view(orders);
  }
  expire(now = Date.now()) {
    for (const [key, value] of this.transitions) {
      if (now - value.since >= FRESHNESS_MS) {
        this.transitions.delete(key);
        this.removed.delete(key);
      }
    }
  }
  get nextExpiry() {
    let at = Infinity;
    for (const value of this.transitions.values())
      at = Math.min(at, value.since + FRESHNESS_MS);
    return at;
  }
  view(orders: Order[]) {
    const displayOrders = orders.slice();
    this.removedPositions = [];
    for (const { order, index } of [...this.removed.values()].sort(
      (a, b) => a.index - b.index,
    )) {
      const position = Math.min(index, displayOrders.length);
      for (let i = 0; i < this.removedPositions.length; i++)
        if (this.removedPositions[i] >= position) this.removedPositions[i]++;
      this.removedPositions.push(position);
      displayOrders.splice(position, 0, order);
    }
    return { displayOrders, activity: new Map(this.transitions) };
  }
  /** Retained grey cards occupy display rows, but not live pagination offsets. */
  liveRange(start: number, end: number) {
    let liveStart = start,
      liveEnd = end;
    for (const position of this.removedPositions) {
      if (position < start) liveStart--;
      if (position < end) liveEnd--;
    }
    return { start: liveStart, end: liveEnd };
  }
}
