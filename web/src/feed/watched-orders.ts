import type { components } from "../gen/api.ts";
import { orderKey } from "../orderbook/order-activity.ts";
import type { LiveFrame } from "../ws-protocol.ts";
type Result = components["schemas"]["OrderResultResponse"];
type Snapshot = Extract<LiveFrame, { type: "orders.snapshot" }>;
type Changed = Extract<LiveFrame, { type: "orders.changed" }>;
export class WatchedOrders {
  private result?: Result;
  private sequence?: number;
  clear() {
    this.result = undefined;
    this.sequence = undefined;
  }
  apply(frame: Snapshot | Changed): Result {
    if (frame.type === "orders.snapshot") {
      if (
        this.sequence === undefined
          ? frame.sequence !== 0
          : !frame.reset || frame.sequence !== this.sequence + 1
      )
        throw new Error("Unexpected order snapshot sequence");
      this.sequence = frame.sequence;
      return (this.result = frame.result);
    }
    if (!this.result || frame.base_sequence !== this.sequence)
      throw new Error("Order subscription sequence gap");
    const rows = new Map(this.result.data.map((row) => [orderKey(row), row]));
    for (const row of frame.removed)
      rows.delete(JSON.stringify([row.venue_id, row.id]));
    for (const row of frame.upserts) {
      if (
        (this.result.range.start &&
          row.sort_key < this.result.range.start.sort_key) ||
        (this.result.range.end && row.sort_key > this.result.range.end.sort_key)
      )
        throw new Error("Order outside subscribed coverage");
      rows.set(orderKey(row), row);
    }
    const data = [...rows.values()].sort((a, b) =>
      a.sort_key < b.sort_key ? -1 : a.sort_key > b.sort_key ? 1 : 0,
    );
    if (new Set(data.map((row) => row.sort_key)).size !== data.length)
      throw new Error("Duplicate order rank");
    this.sequence = frame.sequence;
    return (this.result = { ...this.result, data });
  }
}
