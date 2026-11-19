import { liveSocket } from "../live-socket.ts";
import { LoadedOrders } from "./loaded-orders.ts";
import type { Order, MarketStatus } from "../market.ts";
import { OrderActivityTracker, orderKey } from "../orderbook/order-activity.ts";
import {
  emptyFeed,
  type FeedController,
  type FeedState,
  type Bookmark,
} from "./types.ts";
import {
  LiveRanges,
  FrozenReads,
  type OrderData,
  type OrderQuery,
  type RangeSource,
} from "./range-source.ts";
export type RangeSelection = {
  filters: OrderQuery["filters"];
  side?: "ask" | "bid";
  sort?: OrderQuery["sort"];
};
/** Owns navigation and visual state; the server owns ordering and range reconciliation. */
export class RangeController implements FeedController {
  private state = emptyFeed();
  private activity = new OrderActivityTracker();
  private source: RangeSource;
  private unwatchStatus?: () => void;
  private availability: MarketStatus = "unavailable";
  private current?: OrderData;
  private activeStart = 0;
  private cache: LoadedOrders;
  private projected = new WeakMap<OrderData["data"][number], Order>();
  private anchor?: string;
  private visibleKey?: string;
  private requested?: string;
  private navigation: "around" | "after" | "before" = "around";
  private movingTo?: string;
  private working = false;
  private closed = false;
  private expiry?: ReturnType<typeof setTimeout>;
  private movement?: ReturnType<typeof setTimeout>;
  ready = false;
  constructor(
    private selection: RangeSelection,
    private frozen: boolean,
    private changed: (state: FeedState) => void,
    bookmark?: Bookmark,
  ) {
    this.cache =
      bookmark?.kind === "range"
        ? (bookmark.cache ?? new LoadedOrders())
        : new LoadedOrders();
    this.requested = bookmark?.kind === "range" ? bookmark.anchor : undefined;
    const callbacks = {
      result: (data: OrderData) => this.accept(data),
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
      ? new FrozenReads(callbacks)
      : new LiveRanges(callbacks);
  }
  get bookmark(): Bookmark {
    return {
      kind: "range",
      anchor: this.anchor ?? this.requested,
      cache: this.cache,
    };
  }
  start() {
    if (!this.frozen)
      this.unwatchStatus = liveSocket.watchStatus((status) => {
        this.availability = status;
        this.emit({ status: this.marketLabel() });
      });
    this.working = true;
    this.emit({
      loadingMore: true,
      status: this.frozen ? "Loading" : "Connecting",
    });
    this.source.start(this.query());
  }
  close() {
    if (this.closed) return;
    this.closed = true;
    this.source.close();
    this.unwatchStatus?.();
    clearTimeout(this.expiry);
    clearTimeout(this.movement);
  }
  loadMore() {
    if (!this.working && !this.state.error && this.state.hasMore) this.move();
  }
  setVisibleRange(start: number, end: number, anchor = start) {
    const range = this.activity.liveRange(start, end);
    const point = this.activity.liveRange(anchor, anchor).start;
    const row = this.cache.rows[point];
    if (row) {
      this.anchor = row.key;
      this.visibleKey = orderKey(row);
      this.source.remember?.({
        ...this.query(),
        navigation: { type: "around", key: row.key },
      });
    }
    if (!this.current || this.working || this.state.error) return;
    clearTimeout(this.movement);
    const activeStart = this.activeStart;
    const activeEnd = activeStart + this.current.data.length;
    if (
      (!this.current.data.length && row !== undefined) ||
      (this.current.range.start !== null && point < activeStart + 25) ||
      (this.current.range.end !== null && range.end > activeEnd - 25)
    )
      this.movement = setTimeout(() => this.move(), 80);
  }
  private query(): OrderQuery {
    const { filters, side, sort } = this.selection;
    return {
      filters,
      source: side === "bid" ? "bids" : "asks",
      sort,
      navigation: this.requested
        ? { type: this.navigation, key: this.requested }
        : { type: "head" },
      limit: 150,
      period: null,
    };
  }
  private move() {
    if (!this.current || this.working || this.closed) return;
    const anchor =
      this.anchor ??
      this.current.data[0]?.key ??
      this.current.range.end ??
      this.current.range.start;
    if (!anchor || anchor === this.requested) return;
    this.navigation =
      !this.current.data.length && !this.cache.rows.length
        ? this.current.range.end
          ? "after"
          : "before"
        : "around";
    this.requested = anchor;
    this.movingTo = this.visibleKey;
    this.working = true;
    this.activity.rebase();
    this.emit({ loadingMore: true, error: "" });
    this.source.move(this.query());
  }
  private accept(current: OrderData) {
    if (this.closed) return;
    if (
      this.working &&
      this.movingTo &&
      this.visibleKey !== this.movingTo &&
      this.visibleKey &&
      !current.data.some((row) => orderKey(row) === this.visibleKey)
    ) {
      this.working = false;
      this.move();
      return;
    }
    this.movingTo = undefined;
    this.cache.merge(current);
    this.current = current;
    this.activeStart = Math.max(
      0,
      this.cache.rows.findIndex((row) => row.key === current.data[0]?.key),
    );
    this.working = false;
    this.ready = true;
    this.anchor =
      this.cache.rows.find((row) => orderKey(row) === this.visibleKey)?.key ??
      current.data[0]?.key;
    if (this.requested && this.anchor) {
      this.requested = this.anchor;
      this.source.remember?.(this.query());
    }
    const orders = this.cache.rows.map((entry) => {
      let order = this.projected.get(entry);
      if (!order) {
        const { key: _, ...data } = entry;
        order = data;
        this.projected.set(entry, order);
      }
      return order;
    });
    if (this.frozen) this.activity.rebase();
    this.emit({
      orders,
      ...this.activity.update(orders),

      hasMore: this.cache.hasAfter,
      loadingMore: false,
      error: "",
      status: this.frozen ? "Paused" : this.marketLabel(),
    });
    this.scheduleExpiry();
    if (
      !current.data.length &&
      !this.cache.rows.length &&
      (current.range.start || current.range.end)
    ) {
      clearTimeout(this.movement);
      this.movement = setTimeout(() => this.move(), 80);
    }
  }
  private marketLabel() {
    return this.availability === "available"
      ? "Live"
      : this.availability === "delayed"
        ? "Updates delayed"
        : "Unavailable";
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
