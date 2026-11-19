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
import { LiveRanges, type OrderData, type OrderQuery } from "./range-source.ts";
export type RangeSelection = {
  filters: OrderQuery["filters"];
  side?: "ask" | "bid";
  sort?: OrderQuery["sort"];
};
const PAGE_SIZE = 100;
const WATCH_SIZE = 80;
const RESTART_PAGE = Symbol("Watch baseline replaced");
/** Retain visited pages; query missing data and watch known viewport bounds. */
export class RangeController implements FeedController {
  private state = emptyFeed();
  private activity = new OrderActivityTracker();
  private source: LiveRanges;
  private unwatchStatus?: () => void;
  private availability: MarketStatus = "unavailable";
  private cache: LoadedOrders;
  private projected = new WeakMap<OrderData["data"][number], Order>();
  private anchor?: string;
  private visibleKey?: string;
  private end?: string | null;
  private liveView?: OrderData;
  private watchError = "";
  private pageError = "";
  private watchRange?: OrderData["range"];
  private closed = false;
  private pending?: AbortController;
  private expiry?: ReturnType<typeof setTimeout>;
  private movement?: ReturnType<typeof setTimeout>;
  private scan?: ReturnType<typeof setTimeout>;
  private retry?: ReturnType<typeof setTimeout>;
  private failures = 0;
  ready = false;
  constructor(
    private selection: RangeSelection,
    private changed: (state: FeedState) => void,
    bookmark?: Bookmark,
  ) {
    this.cache =
      bookmark?.kind === "range"
        ? (bookmark.cache ?? new LoadedOrders())
        : new LoadedOrders();
    this.anchor = bookmark?.kind === "range" ? bookmark.anchor : undefined;
    this.end = bookmark?.kind === "range" ? bookmark.end : undefined;
    const callbacks = {
      result: (data: OrderData, requested?: OrderData["range"]) => {
        if (this.closed) return;
        this.watchError = "";
        if (requested) {
          this.pending?.abort(RESTART_PAGE);
          this.cover(data, requested);
          if (this.end === null) this.pageError = "";
        }
        this.liveView = data;
        this.watchRange = data.range;
        this.accept(data);
        if (requested && this.end && !this.cache.rows.length)
          this.scanEmpty(this.end);
        if (data.data.length >= 90) this.watchKnown();
      },
      status: (status: string) => {
        this.activity.rebase();
        this.emit({ status });
      },
      error: (error: unknown) => {
        clearTimeout(this.movement);
        clearTimeout(this.scan);
        this.watchError =
          error instanceof Error ? error.message : String(error);
        // The socket owns subscription backoff; prepare narrower coverage for its next attempt.
        this.source.remember(this.query(this.initialRange(), WATCH_SIZE));
        this.failed(error);
      },
      reset: () => this.activity.rebase(),
    };
    this.source = new LiveRanges(callbacks);
  }
  get bookmark(): Bookmark {
    return {
      kind: "range",
      anchor: this.anchor,
      end: this.end,
      cache: this.cache,
    };
  }
  start() {
    this.unwatchStatus = liveSocket.watchStatus((status) => {
      this.availability = status;
      this.emit({ status: this.marketLabel() });
    });
    this.seedWatch();
  }
  close() {
    this.closed = true;
    this.pending?.abort();
    this.source.close();
    this.unwatchStatus?.();
    clearTimeout(this.expiry);
    clearTimeout(this.movement);
    clearTimeout(this.scan);
    clearTimeout(this.retry);
  }
  loadMore() {
    if (!this.pending && !this.state.error && this.state.hasMore) {
      const start = this.end ?? null;
      void this.load({ start, end: null });
    }
  }
  setVisibleRange(start: number, end: number, anchor = start) {
    const point = this.activity.liveRange(anchor, anchor).start;
    const row = this.cache.rows[point];
    if (!row) return;
    this.anchor = row.key;
    this.visibleKey = orderKey(row);
    this.source.remember(this.query(this.initialRange(), WATCH_SIZE));
    if (!this.ready || this.pending || this.state.error) return;
    clearTimeout(this.movement);
    const visible = this.activity.liveRange(start, end);
    const first = this.cache.rows[visible.start],
      last =
        this.cache.rows[Math.min(visible.end - 1, this.cache.rows.length - 1)];
    if (
      !this.watchRange ||
      (first &&
        this.watchRange.start !== null &&
        first.key < this.watchRange.start) ||
      (last && this.watchRange.end !== null && last.key >= this.watchRange.end)
    ) {
      this.movement = setTimeout(() => {
        this.watchKnown();
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
    this.source.watch(this.query(this.initialRange(), WATCH_SIZE));
  }
  private cover(data: OrderData, requested: OrderData["range"]) {
    const tail = this.cache.rows.at(-1)?.key;
    if (requested.end !== null) return;
    const end = data.range.end;
    if (
      this.end === undefined ||
      end === null ||
      (this.end !== null ? end > this.end : !tail || end > tail)
    )
      this.end = end;
  }
  private scanEmpty(start: string) {
    clearTimeout(this.scan);
    const count = this.cache.rows.length;
    this.scan = setTimeout(() => {
      this.scan = undefined;
      if (
        this.end === start &&
        this.cache.rows.length === count &&
        !this.state.error
      )
        void this.load({ start, end: null });
    }, 80);
  }
  private retryPage(delay: number) {
    clearTimeout(this.retry);
    this.retry = setTimeout(() => {
      this.retry = undefined;
      if (
        !this.closed &&
        !this.pending &&
        this.end !== null &&
        !this.watchError
      )
        void this.load({ start: this.end ?? null, end: null });
    }, delay);
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
      end: end === rows.length ? (this.end ?? null) : rows[end].key,
    };
  }
  private watchKnown() {
    if (this.closed || !this.cache.rows.length) return;
    const range = this.knownRange();
    if (JSON.stringify(range) === JSON.stringify(this.watchRange)) return;
    this.watchRange = range;
    this.source.watch(this.query(range, WATCH_SIZE));
  }
  private async load(range: OrderData["range"]) {
    if (this.closed || this.pending) return;
    clearTimeout(this.retry);
    const stop = new AbortController();
    this.pending = stop;
    this.pageError = "";
    this.activity.rebase();
    this.emit({
      loadingMore: true,
      error: this.watchError,
      status: "Loading",
    });
    try {
      const page = await this.source.query(this.query(range), stop.signal);
      if (this.closed || stop.signal.aborted) return;
      this.cover(page, range);
      this.failures = 0;
      this.accept(page);
      if (!page.data.length && page.range.end !== range.end && page.range.end)
        this.scanEmpty(page.range.end);
    } catch (error) {
      if (this.closed || stop.signal.aborted) return;
      this.pageError = error instanceof Error ? error.message : String(error);
      this.failed(error);
      if (
        !(error instanceof ApiFailure) ||
        ["unavailable", "timed_out", "capacity_exceeded"].includes(error.code)
      )
        this.retryPage(retryDelay(this.failures++));
    } finally {
      if (this.pending === stop) {
        this.pending = undefined;
        this.emit({ loadingMore: false });
        if (!this.closed && stop.signal.reason === RESTART_PAGE)
          this.retryPage(80);
      }
    }
  }
  private failed(error: unknown) {
    this.emit({
      error:
        this.watchError ||
        this.pageError ||
        (error instanceof Error ? error.message : String(error)),
      status: "Unavailable",
      loadingMore: false,
    });
  }
  private accept(data: OrderData) {
    if (this.closed) return;
    this.cache.merge(data);
    // A finite read can arrive after a newer live update; commit both atomically with live data winning.
    if (this.liveView && data !== this.liveView)
      this.cache.merge(this.liveView);
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
    this.emit({
      orders,
      ...this.activity.update(orders),
      hasMore: this.end !== null,
      loadingMore: !!this.pending,
      error: this.watchError || this.pageError,
      status:
        this.watchError || this.pageError ? "Unavailable" : this.marketLabel(),
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
