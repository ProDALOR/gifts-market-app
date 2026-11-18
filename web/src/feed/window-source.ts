import { liveSocket } from "../live-socket.ts";
import { request, ApiFailure } from "../market.ts";
import type { components } from "../gen/api.ts";
export type WindowQuery = components["schemas"]["GiftOrderWindowQuery"];
export type WindowData = components["schemas"]["OrderWindowResponse"];
export type Sync = components["schemas"]["SyncStatus"];
type Snapshot = components["schemas"]["SnapshotWindowResponse"];
export type Callbacks = {
  window: (window: WindowData, sync: Sync) => void;
  sync: (sync: Sync) => void;
  status: (status: string) => void;
  error: (error: unknown) => void;
  reset: () => void;
};
export interface WindowSource {
  start(query: WindowQuery): void;
  move(query: WindowQuery): void;
  remember?(query: WindowQuery): void;
  close(): void;
}
const command = (query: WindowQuery) => ({
  type: "subscribe_window" as const,
  id: "offers",
  query,
});
export class LiveWindows implements WindowSource {
  private unsubscribe?: () => void;
  constructor(private callbacks: Callbacks) {}
  start(query: WindowQuery) {
    this.unsubscribe = liveSocket.subscribe(
      command(query),
      (frame) => {
        if (frame.type === "window")
          this.callbacks.window(frame.window, frame.sync);
        else if (frame.type === "sync") this.callbacks.sync(frame.sync);
        else if (frame.type === "error")
          this.callbacks.error(
            new ApiFailure(frame.error.code, frame.error.message),
          );
      },
      this.callbacks.status,
    );
  }
  move(query: WindowQuery) {
    liveSocket.update(command(query));
  }
  remember(query: WindowQuery) {
    liveSocket.remember(command(query));
  }
  close() {
    this.unsubscribe?.();
  }
}
const release = (snapshot: string) => {
  void fetch("/orders/window", {
    method: "DELETE",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ snapshot }),
    keepalive: true,
  }).catch(() => {});
};
/** Owns one bounded, expiring snapshot lease; ordering anchors stay portable. */
export class FrozenWindows implements WindowSource {
  private snapshot?: string;
  private pending?: AbortController;
  private closed = false;
  constructor(private callbacks: Callbacks) {}
  start(query: WindowQuery) {
    this.move(query);
  }
  move(query: WindowQuery) {
    void this.read(query);
  }
  private async read(query: WindowQuery) {
    this.pending?.abort();
    const attempt = new AbortController();
    this.pending = attempt;
    let restarted = false;
    while (!this.closed && !attempt.signal.aborted) {
      try {
        const result = await request<Snapshot>(
          "/orders/window",
          { query, snapshot: this.snapshot ?? null },
          attempt.signal,
        );
        if (this.closed || this.pending !== attempt || attempt.signal.aborted) {
          if (result.snapshot !== this.snapshot || this.closed)
            release(result.snapshot);
          return;
        }
        this.snapshot = result.snapshot;
        this.callbacks.window(result.window, result.sync);
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
