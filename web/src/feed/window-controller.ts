import { OrderActivityTracker, orderKey } from "../orderbook/order-activity.ts";
import {
  emptyFeed,
  type FeedController,
  type FeedState,
  type Bookmark,
} from "./types.ts";
import {
  LiveWindows,
  FrozenWindows,
  type WindowData,
  type WindowQuery,
  type WindowSource,
  type Sync,
} from "./window-source.ts";
export type WindowSelection = Pick<WindowQuery, "filters" | "side" | "sort">;
/** Owns navigation and visual state; the server owns ordering and window reconciliation. */
export class WindowController implements FeedController {
  private state = emptyFeed();
  private activity = new OrderActivityTracker();
  private source: WindowSource;
  private window?: WindowData;
  private anchor?: string;
  private visibleKey?: string;
  private requested?: string;
  private movingTo?: string;
  private working = false;
  private closed = false;
  private expiry?: ReturnType<typeof setTimeout>;
  private movement?: ReturnType<typeof setTimeout>;
  ready = false;
  constructor(
    private selection: WindowSelection,
    private frozen: boolean,
    private changed: (state: FeedState) => void,
    bookmark?: Bookmark,
  ) {
    this.requested = bookmark?.kind === "window" ? bookmark.anchor : undefined;
    const callbacks = {
      window: (data: WindowData, sync: Sync) => this.accept(data, sync),
      sync: (sync: Sync) =>
        this.emit({ sync, status: sync.state === "live" ? "Live" : "Stale" }),
      status: (status: string) => {
        this.requested = this.anchor ?? this.requested;
        this.activity.rebase();
        this.working = true;
        this.emit({ status, loadingMore: true });
      },
      error: (error: unknown) => {
        this.working = false;
        this.emit({
          error: error instanceof Error ? error.message : String(error),
          status: "Unavailable",
          loadingMore: false,
        });
      },
      reset: () => this.activity.rebase(),
    };
    this.source = frozen
      ? new FrozenWindows(callbacks)
      : new LiveWindows(callbacks);
  }
  get bookmark(): Bookmark {
    return { kind: "window", anchor: this.anchor ?? this.requested };
  }
  start() {
    this.working = true;
    this.emit({
      loadingMore: true,
      status: this.frozen ? "Loading" : "Connecting",
    });
    this.source.start(this.query());
  }
  close() {
    this.closed = true;
    this.source.close();
    clearTimeout(this.expiry);
    clearTimeout(this.movement);
  }
  loadMore() {
    if (!this.working && !this.state.error && this.state.hasMore) this.move();
  }
  setVisibleRange(start: number, end: number, anchor = start) {
    const range = this.activity.liveRange(start, end);
    const point = this.activity.liveRange(anchor, anchor).start;
    const row = this.window?.data[point];
    if (row) {
      this.anchor = row.position;
      this.visibleKey = orderKey(row);
      this.source.remember?.({ ...this.query(), anchor: row.position });
    }
    if (!this.window || this.working || this.state.error) return;
    clearTimeout(this.movement);
    if (
      (this.window.has_before && point < 25) ||
      (this.window.has_after && range.end > this.window.data.length - 25)
    )
      this.movement = setTimeout(() => this.move(), 80);
  }
  private query(): WindowQuery {
    const { filters, side, sort } = this.selection;
    return {
      filters,
      side,
      sort,
      anchor: this.requested ?? null,
      before: 50,
      after: 99,
    };
  }
  private move() {
    if (!this.window || this.working || this.closed) return;
    const anchor = this.anchor ?? this.window.data[0]?.position;
    if (!anchor || anchor === this.requested) return;
    this.requested = anchor;
    this.movingTo = this.visibleKey;
    this.working = true;
    this.activity.rebase();
    this.emit({ loadingMore: true, error: "" });
    this.source.move(this.query());
  }
  private accept(window: WindowData, sync: Sync) {
    if (this.closed) return;
    if (
      this.working &&
      this.movingTo &&
      this.visibleKey !== this.movingTo &&
      this.visibleKey &&
      !window.data.some((row) => orderKey(row) === this.visibleKey)
    ) {
      this.working = false;
      this.move();
      return;
    }
    this.movingTo = undefined;
    this.window = window;
    this.working = false;
    this.ready = true;
    if (this.requested && window.anchor) {
      this.requested = window.anchor;
      this.source.remember?.(this.query());
    }
    this.anchor =
      window.data.find((row) => orderKey(row) === this.visibleKey)?.position ??
      window.anchor ??
      undefined;
    const previous = new Map(
      this.state.orders.map((row) => [orderKey(row), row]),
    );
    const orders = window.data.map(({ position: _, ...row }) => {
      const old = previous.get(orderKey(row));
      return old && JSON.stringify(old) === JSON.stringify(row) ? old : row;
    });
    if (this.frozen) this.activity.rebase();
    this.emit({
      orders,
      ...this.activity.update(orders, {
        before: window.has_before,
        after: window.has_after,
      }),
      sync,
      cursor: window.cursor,
      hasMore: window.has_after,
      loadingMore: false,
      error: "",
      status: this.frozen ? "Paused" : sync.state === "live" ? "Live" : "Stale",
    });
    this.scheduleExpiry();
  }
  private scheduleExpiry() {
    clearTimeout(this.expiry);
    const at = this.activity.nextExpiry;
    if (Number.isFinite(at))
      this.expiry = setTimeout(
        () => {
          this.activity.expire();
          this.emit(this.activity.view(this.state.orders));
          this.scheduleExpiry();
        },
        Math.max(0, at - Date.now()),
      );
  }
  private emit(patch: Partial<FeedState>) {
    if (!this.closed) {
      this.state = { ...this.state, ...patch };
      this.changed(this.state);
    }
  }
}
