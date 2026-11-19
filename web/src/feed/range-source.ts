import { retryDelay } from "../retry.ts";
import { WatchedOrders } from "./watched-orders.ts";
import { liveSocket } from "../live-socket.ts";
import { request, ApiFailure } from "../market.ts";
import type { components } from "../gen/api.ts";
export type OrderQuery = components["schemas"]["GiftOrderQuery"];
export type OrderData = components["schemas"]["OrderReadResponse"];
export type Sync = components["schemas"]["SyncStatus"];
type Snapshot = components["schemas"]["OrderReadResponse"];
export type Callbacks = {
  result: (result: OrderData) => void;
  status: (status: string) => void;
  error: (error: unknown) => void;
  reset: () => void;
};
export interface RangeSource {
  start(query: OrderQuery): void;
  move(query: OrderQuery): void;
  remember?(query: OrderQuery): void;
  close(): void;
}
const command = (query: OrderQuery) => ({
  type: "orders.watch" as const,
  id: "offers",
  params: { query, range: null },
});
export class LiveRanges implements RangeSource {
  private unsubscribe?: () => void;
  private state = new WatchedOrders();
  private recovery?: ReturnType<typeof setTimeout>;
  private failures = 0;
  constructor(private callbacks: Callbacks) {}
  private clear() {
    clearTimeout(this.recovery);
    this.recovery = undefined;
    this.state.clear();
  }
  start(query: OrderQuery) {
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
            this.callbacks.reset();
          }
          this.callbacks.result(result);
        } else if (frame.type === "error") {
          this.clear();
          this.callbacks.error(
            new ApiFailure(frame.error.code, frame.error.message),
          );
        }
      },
      (status) => {
        this.clear();
        this.callbacks.status(status);
      },
    );
  }
  move(query: OrderQuery) {
    this.clear();
    liveSocket.update(command(query));
  }
  remember(query: OrderQuery) {
    liveSocket.remember(command(query));
  }
  close() {
    this.unsubscribe?.();
    this.clear();
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
/** Owns one bounded, expiring snapshot lease; ordering anchors stay portable. */
export class FrozenReads implements RangeSource {
  private snapshot?: string;
  private pending?: AbortController;
  private closed = false;
  constructor(private callbacks: Callbacks) {}
  start(query: OrderQuery) {
    this.move(query);
  }
  move(query: OrderQuery) {
    void this.read(query);
  }
  private async read(query: OrderQuery) {
    this.pending?.abort();
    const attempt = new AbortController();
    this.pending = attempt;
    let restarted = false;
    while (!this.closed && !attempt.signal.aborted) {
      try {
        const result = await request<Snapshot>(
          "/orders/query",
          { ...query, view: { kind: "snapshot", id: this.snapshot ?? null } },
          attempt.signal,
        );
        if (!result.snapshot)
          throw new Error("Snapshot lease missing from frozen read");
        if (this.closed || this.pending !== attempt || attempt.signal.aborted) {
          if (result.snapshot !== this.snapshot || this.closed)
            release(result.snapshot);
          return;
        }
        this.snapshot = result.snapshot;
        this.callbacks.result(result);
        return;
      } catch (error) {
        if (this.closed || attempt.signal.aborted) return;
        if (
          error instanceof ApiFailure &&
          error.code === "restart_read" &&
          !restarted
        ) {
          restarted = true;
          if (this.snapshot) release(this.snapshot);
          this.snapshot = undefined;
          this.callbacks.reset();
          continue;
        }
        this.callbacks.error(error);
        return;
      }
    }
  }
  close() {
    this.closed = true;
    this.pending?.abort();
    if (this.snapshot) {
      release(this.snapshot);
      this.snapshot = undefined;
    }
  }
}
