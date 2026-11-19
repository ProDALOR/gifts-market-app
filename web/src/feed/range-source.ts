import { retryDelay } from "../retry.ts";
import { WatchedOrders } from "./watched-orders.ts";
import { liveSocket } from "../live-socket.ts";
import { request, ApiFailure, type OrderQuery } from "../market.ts";
import type { components } from "../gen/api.ts";
export type OrderData = components["schemas"]["OrderSnapshotResponse"];
export type Snapshot = components["schemas"]["OrderReadResponse"];
export type Callbacks = {
  result: (result: OrderData, requested?: OrderData["range"]) => void;
  status: (status: "connecting" | "reconnecting") => void;
  error: (error: unknown) => void;
};
/** A page may be empty, but partial coverage must advance within the requested bounds. */
export function checkedRange<T extends OrderData>(
  read: T,
  requested: OrderData["range"],
): T {
  const { start, end } = read.range;
  if (
    start !== requested.start ||
    (end !== requested.end &&
      (end === null ||
        (start !== null && end <= start) ||
        (requested.end !== null && end > requested.end)))
  )
    throw new Error("Market returned invalid range coverage");
  return read;
}
const command = (query: OrderQuery) => ({
  type: "orders.watch" as const,
  id: "offers",
  params: query,
});
/** Finite reads and persistent watches share the existing WebSocket connection. */
export class LiveRanges {
  private unsubscribe?: () => void;
  private closed = false;
  private state = new WatchedOrders();
  private recovery?: ReturnType<typeof setTimeout>;
  private failures = 0;
  constructor(private callbacks: Callbacks) {}
  async query(query: OrderQuery, signal: AbortSignal) {
    const read = await liveSocket.request(
      { method: "orders.query", params: query },
      signal,
    );
    return checkedRange(read, query.range ?? { start: null, end: null });
  }
  watch(query: OrderQuery) {
    if (this.closed) return;
    if (this.unsubscribe) {
      if (liveSocket.update(command(query))) {
        this.state.clear();
        clearTimeout(this.recovery);
        this.recovery = undefined;
      }
      return;
    }
    this.unsubscribe = liveSocket.subscribe(
      command(query),
      (frame, subscription) => {
        if (this.closed) return;
        if (
          frame.type === "orders.snapshot" ||
          frame.type === "orders.changed"
        ) {
          let result: OrderData;
          const requested =
            subscription.type === "orders.watch"
              ? (subscription.params.range ?? { start: null, end: null })
              : undefined;
          try {
            result = this.state.apply(frame);
            if (frame.type === "orders.snapshot" && requested)
              checkedRange(result, requested);
          } catch {
            this.state.clear();
            this.callbacks.status("reconnecting");
            this.recovery ??= setTimeout(() => {
              this.recovery = undefined;
              liveSocket.retry("offers");
            }, retryDelay(this.failures++));
            return;
          }
          if (frame.type === "orders.snapshot") {
            clearTimeout(this.recovery);
            this.recovery = undefined;
            this.failures = 0;
          }
          this.callbacks.result(
            result,
            frame.type === "orders.snapshot" ? requested : undefined,
          );
        } else if (frame.type === "error") {
          clearTimeout(this.recovery);
          this.recovery = undefined;
          this.state.clear();
          this.callbacks.error(
            new ApiFailure(frame.error.code, frame.error.message),
          );
        }
      },
      (status) => {
        if (this.closed) return;
        clearTimeout(this.recovery);
        this.recovery = undefined;
        this.state.clear();
        this.callbacks.status(
          status === "Reconnecting" ? "reconnecting" : "connecting",
        );
      },
    );
  }
  remember(query: OrderQuery) {
    if (this.closed) return;
    liveSocket.remember(command(query));
  }
  close() {
    this.closed = true;
    this.unsubscribe?.();
    clearTimeout(this.recovery);
    this.state.clear();
  }
}
export const release = (snapshot: string) => {
  void fetch("/orders/query", {
    method: "DELETE",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ snapshot }),
    keepalive: true,
  }).catch(() => {});
};
/** One expiring immutable view, reused by all paused range reads. */
export class FrozenReads {
  private snapshot?: string;
  private closed = false;
  async query(query: OrderQuery, signal: AbortSignal): Promise<Snapshot> {
    if (this.closed || signal.aborted)
      throw signal.reason ?? new Error("Read cancelled");
    try {
      const result = await request<Snapshot>(
        "/orders/query",
        {
          ...query,
          view: { kind: "snapshot", id: this.snapshot ?? null },
        },
        signal,
      );
      if (!result.snapshot)
        throw new Error("Snapshot lease missing from frozen read");
      if (this.closed || signal.aborted) {
        if (result.snapshot !== this.snapshot || this.closed)
          release(result.snapshot);
        throw signal.reason ?? new Error("Read cancelled");
      }
      this.snapshot = result.snapshot;
      return checkedRange(result, query.range ?? { start: null, end: null });
    } catch (error) {
      if (
        error instanceof ApiFailure &&
        error.code === "restart_read" &&
        !signal.aborted
      ) {
        if (this.snapshot) release(this.snapshot);
        this.snapshot = undefined;
      }
      throw error;
    }
  }
  close() {
    this.closed = true;
    if (this.snapshot) release(this.snapshot);
    this.snapshot = undefined;
  }
}
