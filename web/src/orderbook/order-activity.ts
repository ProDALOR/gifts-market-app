import type { Order } from "../market.ts";
export const FRESHNESS_MS = 60_000;
const MAX_REMOVED = 50;
export const orderKey = (order: Order) =>
  JSON.stringify([order.book.venue_id, order.id]);
export type OrderActivity = { state: "new" | "removed"; since: number };
type Removed = { order: Order; index: number };
/** Local view transitions; leaving a window's edges is not evidence of removal. */
export class OrderActivityTracker {
  private previous: Order[] = [];
  private transitions = new Map<string, OrderActivity>();
  private removed = new Map<string, Removed>();
  private removedPositions: number[] = [];
  private baseline = true;
  rebase() {
    this.baseline = true;
  }
  update(orders: Order[], now = Date.now()) {
    this.expire(now);
    const present = new Set(orders.map(orderKey));
    const before = this.previous;
    const known = new Set(before.map(orderKey));
    if (this.baseline) {
      this.transitions.clear();
      this.removed.clear();
    } else {
      // The range controller rebases on navigation; missing cached rows are explicit range removals.
      for (const order of orders) {
        const key = orderKey(order);
        if (!known.has(key) || this.transitions.get(key)?.state === "removed")
          this.transitions.set(key, { state: "new", since: now });
      }
      before.forEach((order, index) => {
        const key = orderKey(order);
        if (!present.has(key) && !this.removed.has(key)) {
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
    this.previous = orders.slice();
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
  /** Retained grey cards occupy display rows, but not live window offsets. */
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
