import { retryDelay } from "../retry.ts";
import { WatchedOrders } from "./watched-orders.ts";
import { liveSocket } from "../live-socket.ts";
import { request, ApiFailure } from "../market.ts";
import type { components } from "../gen/api.ts";
export type OrderQuery = components["schemas"]["GiftOrderQuery"];
export type OrderData = components["schemas"]["OrderSnapshotResponse"];
export type Snapshot = components["schemas"]["OrderReadResponse"];
export type Callbacks = {
  result: (result: OrderData, next?: string | null) => void;
  status: (status: string) => void;
  error: (error: unknown) => void;
  reset: () => void;
};
export interface RangeSource {
  query(query: OrderQuery, signal: AbortSignal): Promise<Snapshot>;
  watch?(query: OrderQuery): void;
  close(): void;
}
const command = (query: OrderQuery) => ({
  type: "orders.watch" as const,
  id: "offers",
  params: query,
});
/** Finite reads and persistent watches share the existing WebSocket connection. */
export class LiveRanges implements RangeSource {
  private unsubscribe?: () => void;
  private state = new WatchedOrders();
  private recovery?: ReturnType<typeof setTimeout>;
  private failures = 0;
  private failed = false;
  constructor(private callbacks: Callbacks) {}
  query(query: OrderQuery, signal: AbortSignal) {
    return liveSocket.request(
      { method: "orders.query", params: query },
      signal,
    );
  }
  watch(query: OrderQuery) {
    if (this.unsubscribe) {
      const changed = liveSocket.update(command(query));
      if (changed || this.failed) {
        this.state.clear();
        clearTimeout(this.recovery);
        this.recovery = undefined;
        const retry = !changed && this.failed;
        this.failed = false;
        if (retry) liveSocket.retry("offers");
      }
      return;
    }
    this.unsubscribe = liveSocket.subscribe(
      command(query),
      (frame) => {
        if (
          frame.type === "orders.snapshot" ||
          frame.type === "orders.changed"
        ) {
          let result: OrderData;
          try {
            result = this.state.apply(frame);
          } catch {
            this.state.clear();
            this.callbacks.reset();
            this.callbacks.status("Reconnecting");
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
            this.failed = false;
            this.callbacks.reset();
          }
          this.callbacks.result(
            result,
            frame.type === "orders.snapshot" ? frame.next : undefined,
          );
        } else if (frame.type === "error") {
          this.failed = true;
          this.state.clear();
          this.callbacks.error(
            new ApiFailure(frame.error.code, frame.error.message),
          );
        }
      },
      (status) => {
        this.state.clear();
        this.callbacks.status(status);
      },
    );
  }
  close() {
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
export class FrozenReads implements RangeSource {
  private snapshot?: string;
  private closed = false;
  constructor(private callbacks: Callbacks) {}
  async query(query: OrderQuery, signal: AbortSignal): Promise<Snapshot> {
    let restarted = false;
    while (!this.closed && !signal.aborted) {
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
        return result;
      } catch (error) {
        if (
          error instanceof ApiFailure &&
          error.code === "restart_read" &&
          !restarted &&
          !signal.aborted
        ) {
          restarted = true;
          if (this.snapshot) release(this.snapshot);
          this.snapshot = undefined;
          this.callbacks.reset();
          continue;
        }
        throw error;
      }
    }
    throw signal.reason ?? new Error("Read cancelled");
  }
  close() {
    this.closed = true;
    if (this.snapshot) release(this.snapshot);
    this.snapshot = undefined;
  }
}
