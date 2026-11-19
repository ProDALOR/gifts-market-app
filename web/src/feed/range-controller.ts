import { liveSocket } from "../live-socket.ts";
import { retryDelay } from "../retry.ts";
import { LoadedOrders } from "./loaded-orders.ts";
import { ApiFailure, type Order, type MarketStatus } from "../market.ts";
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
const PAGE_SIZE = 100;
const WATCH_SIZE = 80;
/** Retain visited pages; query missing data and watch known viewport bounds. */
export class RangeController implements FeedController {
  private state = emptyFeed();
  private activity = new OrderActivityTracker();
  private source: RangeSource;
  private unwatchStatus?: () => void;
  private availability: MarketStatus = "unavailable";
  private cache: LoadedOrders;
  private projected = new WeakMap<OrderData["data"][number], Order>();
  private anchor?: string;
  private visibleKey?: string;
  private next?: string | null;
  private watchRange?: OrderData["range"];
  private working = false;
  private overflow = false;
  private closed = false;
  private pending?: AbortController;
  private expiry?: ReturnType<typeof setTimeout>;
  private movement?: ReturnType<typeof setTimeout>;
  private retry?: ReturnType<typeof setTimeout>;
  private failures = 0;
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
    this.anchor = bookmark?.kind === "range" ? bookmark.anchor : undefined;
    this.next = bookmark?.kind === "range" ? bookmark.next : undefined;
    const callbacks = {
      result: (data: OrderData, next?: string | null) => {
        if (this.closed) return;
        if (next !== undefined) this.continueAfter(data, next);
        this.watchRange = data.range;
        this.accept(data);
        if (!this.working) this.emit({ loadingMore: false });
        if (next && !this.cache.rows.length) {
          clearTimeout(this.movement);
          this.movement = setTimeout(() => {
            if (!this.cache.rows.length)
              void this.load({ start: next, end: null });
          }, 80);
        }
        if (data.data.length >= 90) this.watchKnown();
      },
      status: (status: string) => {
        this.activity.rebase();
        this.emit({ status });
      },
      error: (error: unknown) => {
        if (error instanceof ApiFailure && error.code === "capacity_exceeded") {
          this.watchRange = undefined;
          this.overflow = true;
          if (!this.working) {
            this.overflow = false;
            this.seedWatch();
          }
        } else this.failed(error);
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
      anchor: this.anchor,
      next: this.next,
      cache: this.cache,
    };
  }
  start() {
    if (!this.frozen)
      this.unwatchStatus = liveSocket.watchStatus((status) => {
        this.availability = status;
        this.emit({ status: this.marketLabel() });
      });
    if (this.frozen) void this.load(this.initialRange());
    else this.seedWatch();
  }
  close() {
    this.closed = true;
    this.pending?.abort();
    this.source.close();
    this.unwatchStatus?.();
    clearTimeout(this.expiry);
    clearTimeout(this.movement);
    clearTimeout(this.retry);
  }
  loadMore() {
    if (!this.working && !this.state.error && this.state.hasMore) {
      const start = this.next ?? this.cache.rows.at(-1)?.key ?? null;
      void this.load({ start, end: null });
    }
  }
  setVisibleRange(start: number, end: number, anchor = start) {
    const point = this.activity.liveRange(anchor, anchor).start;
    const row = this.cache.rows[point];
    if (!row) return;
    this.anchor = row.key;
    this.visibleKey = orderKey(row);
    if (!this.ready || this.working || this.state.error) return;
    clearTimeout(this.movement);
    const visible = this.activity.liveRange(start, end);
    const first = this.cache.rows[visible.start],
      last = this.cache.rows[Math.min(visible.end, this.cache.rows.length - 1)];
    if (
      !this.watchRange ||
      (first &&
        this.watchRange.start !== null &&
        first.key <= this.watchRange.start) ||
      (last && this.watchRange.end !== null && last.key >= this.watchRange.end)
    ) {
      this.movement = setTimeout(() => {
        if (this.frozen) void this.load(this.knownRange());
        else this.watchKnown();
      }, 80);
    }
  }
  private query(range: OrderData["range"], limit = PAGE_SIZE): OrderQuery {
    return {
      filters: this.selection.filters,
      source: this.selection.side === "bid" ? "bids" : "asks",
      sort: this.selection.sort,
      range,
      limit,
    };
  }
  private seedWatch() {
    if (this.closed) return;
    this.watchRange = undefined;
    this.activity.rebase();
    this.emit({ loadingMore: true, error: "", status: "Connecting" });
    this.source.watch?.(this.query(this.initialRange(), WATCH_SIZE));
  }
  private continueAfter(data: OrderData, next: string | null) {
    // A null continuation on a bounded viewport says nothing about the market tail.
    const tail = this.cache.rows.at(-1)?.key;
    if (
      data.range.end === null ||
      (next !== null && (!tail || data.range.end >= tail))
    )
      this.next = next;
  }
  private initialRange(): OrderData["range"] {
    const index = this.cache.rows.findIndex((row) => row.key === this.anchor);
    const start = Math.max(0, index - 20);
    return {
      start:
        start === 0 && !this.cache.hasBefore
          ? null
          : (this.cache.rows[start]?.key ?? null),
      end: null,
    };
  }
  private knownRange(): OrderData["range"] {
    const rows = this.cache.rows;
    const index = Math.max(
      0,
      rows.findIndex((row) =>
        this.visibleKey
          ? orderKey(row) === this.visibleKey
          : row.key === this.anchor,
      ),
    );
    const start = Math.max(0, index - 20),
      end = Math.min(rows.length, start + WATCH_SIZE);
    return {
      start:
        start === 0 && !this.cache.hasBefore
          ? null
          : (rows[start]?.key ?? null),
      end:
        end === rows.length && !this.cache.hasAfter
          ? null
          : (rows[end - 1]?.key ?? this.watchRange?.end ?? null),
    };
  }
  private watchKnown() {
    if (this.closed || this.frozen || !this.cache.rows.length) return;
    const range = this.knownRange();
    if (JSON.stringify(range) === JSON.stringify(this.watchRange)) return;
    this.watchRange = range;
    this.source.watch?.(this.query(range, WATCH_SIZE));
  }
  private async load(range: OrderData["range"]) {
    if (this.closed || this.working) return;
    clearTimeout(this.retry);
    this.working = true;
    this.activity.rebase();
    this.emit({
      loadingMore: true,
      error: "",
      status: this.frozen ? "Loading" : "Connecting",
    });
    const stop = new AbortController();
    this.pending = stop;
    try {
      const page = await this.source.query(this.query(range), stop.signal);
      if (this.closed || stop.signal.aborted) return;
      this.continueAfter(page, page.next);
      this.failures = 0;
      this.accept(page);
      this.watchRange = this.frozen ? page.range : undefined;
      if (!this.frozen && !this.overflow) {
        if (this.cache.rows.length) this.watchKnown();
        else if (!page.next) {
          this.watchRange = page.range;
          this.source.watch?.(this.query(page.range, WATCH_SIZE));
        }
      }
      if (!page.data.length && page.next) {
        this.movement = setTimeout(() => {
          void this.load({ start: page.next, end: range.end });
        }, 80);
      }
    } catch (error) {
      if (this.closed || stop.signal.aborted) return;
      this.failed(error);
      if (
        !(error instanceof ApiFailure) ||
        ["unavailable", "timed_out", "capacity_exceeded"].includes(error.code)
      )
        this.retry = setTimeout(() => {
          void this.load(range);
        }, retryDelay(this.failures++));
    } finally {
      if (this.pending === stop) {
        this.pending = undefined;
        this.working = false;
        this.emit({ loadingMore: false });
        if (this.overflow && !this.closed) {
          this.overflow = false;
          this.seedWatch();
        }
      }
    }
  }
  private failed(error: unknown) {
    this.emit({
      error: error instanceof Error ? error.message : String(error),
      status: "Unavailable",
      loadingMore: false,
    });
  }
  private accept(data: OrderData) {
    if (this.closed) return;
    this.cache.merge(data);
    this.ready = true;
    this.anchor =
      this.cache.rows.find((row) => orderKey(row) === this.visibleKey)?.key ??
      this.anchor ??
      data.data[0]?.key;
    const orders = this.cache.rows.map((entry) => {
      let order = this.projected.get(entry);
      if (!order) {
        const { key: _, ...row } = entry;
        order = row;
        this.projected.set(entry, order);
      }
      return order;
    });
    if (this.frozen) this.activity.rebase();
    this.emit({
      orders,
      ...this.activity.update(orders),
      hasMore: this.cache.hasAfter,
      error: "",
      status: this.frozen ? "Paused" : this.marketLabel(),
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
  private marketLabel() {
    return this.availability === "available"
      ? "Live"
      : this.availability === "delayed"
        ? "Updates delayed"
        : "Unavailable";
  }
  private emit(patch: Partial<FeedState>) {
    if (!this.closed) {
      this.state = { ...this.state, ...patch };
      this.changed(this.state);
    }
  }
}
