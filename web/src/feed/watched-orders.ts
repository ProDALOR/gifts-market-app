import type { components } from "../gen/api.ts";
import { orderKey } from "../orderbook/order-activity.ts";
import type { LiveFrame } from "../ws-protocol.ts";
import { lowerBound } from "./loaded-orders.ts";
type Result = components["schemas"]["OrderSnapshotResponse"];
type Entry = Result["data"][number];
type Snapshot = Extract<LiveFrame, { type: "orders.snapshot" }>;
type Changed = Extract<LiveFrame, { type: "orders.changed" }>;
export class WatchedOrders {
  private result?: Result;
  private sequence?: number;
  private identities = new Map<string, Entry>();
  clear() {
    this.result = undefined;
    this.sequence = undefined;
    this.identities.clear();
  }
  apply(frame: Snapshot | Changed): Result {
    if (frame.type === "orders.snapshot") {
      if (
        this.sequence === undefined
          ? frame.sequence !== 0
          : frame.sequence !== this.sequence + 1
      )
        throw new Error("Unexpected order snapshot sequence");
      this.sequence = frame.sequence;
      this.identities = new Map(frame.data.map((row) => [orderKey(row), row]));
      return (this.result = {
        data: frame.data,
        range: frame.range,
      });
    }
    if (!this.result || frame.sequence !== this.sequence! + 1)
      throw new Error("Order subscription sequence gap");
    // Build the entire batch before committing the snapshot or its lookup index.
    for (const row of frame.upserts)
      if (
        (this.result.range.start !== null &&
          row.key < this.result.range.start) ||
        (this.result.range.end !== null && row.key > this.result.range.end)
      )
        throw new Error("Order outside subscribed coverage");
    const data = this.result.data.slice();
    const remove = (id: string) => {
      const old = this.identities.get(id);
      if (old) {
        const i = lowerBound(data, old.key);
        data.splice(i, 1);
      }
    };
    for (const row of frame.removed)
      remove(JSON.stringify([row.venue_id, row.id]));
    // Remove every affected identity first, so insertion never depends on update order.
    for (const row of frame.upserts) remove(orderKey(row));
    for (const row of frame.upserts) {
      const i = lowerBound(data, row.key);
      if (data[i]?.key === row.key) throw new Error("Duplicate order rank");
      data.splice(i, 0, row);
    }
    for (const row of frame.removed)
      this.identities.delete(JSON.stringify([row.venue_id, row.id]));
    for (const row of frame.upserts) this.identities.set(orderKey(row), row);
    this.sequence = frame.sequence;
    return (this.result = { ...this.result, data });
  }
}
