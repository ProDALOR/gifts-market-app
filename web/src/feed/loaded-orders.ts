import type { components } from "../gen/api.ts";
import { orderKey } from "../orderbook/order-activity.ts";
type Result = components["schemas"]["OrderResultResponse"];
type Entry = Result["data"][number];

/** Visited rows stay available; a server window refreshes its exact ordering range. */
export class LoadedOrders {
  rows: Entry[] = [];
  hasBefore = false;
  hasAfter = true;
  merge(view: Result) {
    const previous = new Map(this.rows.map((row) => [orderKey(row), row]));
    const keys = new Set(view.data.map(orderKey));
    const first = view.range.start?.sort_key;
    const last = view.range.end?.sort_key;
    const retained = this.rows.filter(
      (row) =>
        !keys.has(orderKey(row)) &&
        ((first !== undefined && row.sort_key < first) ||
          (last !== undefined && row.sort_key > last)),
    );
    const incoming = view.data.map((row) => {
      const old = previous.get(orderKey(row));
      return old && JSON.stringify(old) === JSON.stringify(row) ? old : row;
    });
    const merged: Entry[] = [];
    let left = 0,
      right = 0;
    while (left < retained.length && right < incoming.length)
      merged.push(
        retained[left].sort_key < incoming[right].sort_key
          ? retained[left++]
          : incoming[right++],
      );
    this.rows = merged.concat(retained.slice(left), incoming.slice(right));
    if (
      first === undefined ||
      !this.rows.length ||
      this.rows[0].sort_key >= first
    )
      this.hasBefore = first !== undefined;
    if (
      last === undefined ||
      !this.rows.length ||
      this.rows.at(-1)!.sort_key <= last
    )
      this.hasAfter = last !== undefined;
  }
}
