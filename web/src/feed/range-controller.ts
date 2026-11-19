import { liveSocket } from "../live-socket.ts";
import { retryDelay } from "../retry.ts";
import { LoadedOrders } from "./loaded-orders.ts";
import {
  ApiFailure,
  type FeedQuery,
  type OrderQuery,
  type Order,
  type MarketStatus,
} from "../market.ts";
import { OrderActivityTracker, orderKey } from "../orderbook/order-activity.ts";
import {
  emptyFeed,
  type FeedController,
  type FeedState,
  type Bookmark,
} from "./types.ts";
import { LiveRanges, type OrderData } from "./range-source.ts";
import type { OrderRange } from "./coverage.ts";
const PAGE_SIZE = 100;
const WATCH_SIZE = 80;
const RESTART_PAGE = Symbol("Watch baseline replaced");
type PageWait = {
  kind: "continuation" | "backoff";
  timer: ReturnType<typeof setTimeout>;
};
type WatchState =
  | { kind: "connecting" | "reconnecting" }
  | { kind: "live"; view: OrderData }
  | { kind: "failed"; error: string }
  | { kind: "closed" };
/** Cached rows survive navigation; coverage, not the last row, determines unread work. */
export class RangeController implements FeedController {
  private state = emptyFeed();
  private activity = new OrderActivityTracker();
  private source: LiveRanges;
  private watch: WatchState = { kind: "connecting" };
  private unwatchStatus?: () => void;
  private availability: MarketStatus = "unavailable";
  private cache: LoadedOrders;
  private projected = new WeakMap<OrderData["data"][number], Order>();
  private anchor?: string;
  private visibleKey?: string;
  private viewport?: OrderRange;
  private pageError = "";
  private wantPage = false;
  private pending?: AbortController;
  private expiry?: ReturnType<typeof setTimeout>;
  private movement?: ReturnType<typeof setTimeout>;
  private retry?: PageWait;
  private failures = 0;
  ready = false;
  constructor(
    private selection: FeedQuery,
    private changed: (state: FeedState) => void,
    bookmark?: Bookmark,
  ) {
    this.cache = bookmark?.cache ?? new LoadedOrders();
    this.anchor = bookmark?.anchor;
    this.source = new LiveRanges({
      result: (data, requested) => {
        if (this.closed) return;
        if (requested) {
          this.cancelPage();
          this.activity.rebase();
        }
        this.watch = { kind: "live", view: data };
        this.accept(data, requested);
        if (!this.cache.unread) {
          this.pageError = "";
          this.wantPage = false;
          clearTimeout(this.retry?.timer);
          this.retry = undefined;
          this.emit();
        }
        if (data.data.length >= 90) this.watchKnown();
        if (requested && !this.cache.rows.length && this.cache.unread) {
          this.wantPage = true;
          this.retryPage(80);
        } else this.resumePage();
      },
      status: (kind) => {
        if (this.closed) return;
        this.watch = { kind };
        this.cancelPage();
        this.activity.rebase();
        clearTimeout(this.movement);
        this.emit();
      },
      error: (error) => {
        if (this.closed) return;
        this.watch = {
          kind: "failed",
          error: error instanceof Error ? error.message : String(error),
        };
        this.cancelPage();
        clearTimeout(this.movement);
        // Socket-owned backoff retries the latest anchor, never a retired collection.
        this.source.remember(this.query(this.initialRange(), WATCH_SIZE));
        this.emit();
      },
    });
  }
  private get closed() {
    return this.watch.kind === "closed";
  }
  get bookmark(): Bookmark {
    return { anchor: this.anchor, cache: this.cache };
  }
  start() {
    this.unwatchStatus = liveSocket.watchStatus((status) => {
      this.availability = status;
      this.emit();
    });
    this.source.watch(this.query(this.initialRange(), WATCH_SIZE));
  }
  close() {
    this.watch = { kind: "closed" };
    this.pending?.abort();
    this.source.close();
    this.unwatchStatus?.();
    clearTimeout(this.expiry);
    clearTimeout(this.movement);
    clearTimeout(this.retry?.timer);
  }
  loadMore() {
    if (this.closed || this.pageError || this.watch.kind === "failed") return;
    this.wantPage = true;
    if (this.retry?.kind === "continuation") {
      clearTimeout(this.retry.timer);
      this.retry = undefined;
    }
    this.resumePage();
  }
  setVisibleRange(start: number, end: number, anchor = start) {
    if (this.closed) return;
    const point = this.activity.liveRange(anchor, anchor).start;
    const row = this.cache.rows[point];
    if (!row) return;
    this.anchor = row.key;
    this.visibleKey = orderKey(row);
    const visible = this.activity.liveRange(start, end);
    const first = this.cache.rows[visible.start];
    const last =
      this.cache.rows[Math.min(visible.end - 1, this.cache.rows.length - 1)];
    this.viewport =
      first && last ? { start: first.key, end: last.key } : undefined;
    this.source.remember(this.query(this.initialRange(), WATCH_SIZE));
    this.queueViewport();
  }
  private query(range: OrderRange, limit = PAGE_SIZE): OrderQuery {
    return { ...this.selection, range, limit };
  }
  private cancelPage() {
    if (!this.pending) return;
    this.wantPage = true;
    this.pending.abort(RESTART_PAGE);
  }
  private pageRange() {
    return (
      (this.viewport && this.cache.unreadWithin(this.viewport)) ??
      this.cache.unread
    );
  }
  private resumePage() {
    if (
      !this.wantPage ||
      this.pending ||
      this.retry ||
      this.watch.kind !== "live"
    )
      return;
    const range = this.pageRange();
    if (!range) {
      this.wantPage = false;
      return;
    }
    void this.load(range);
  }
  private retryPage(delay: number, kind: PageWait["kind"] = "continuation") {
    clearTimeout(this.retry?.timer);
    this.retry = {
      kind,
      timer: setTimeout(() => {
        this.retry = undefined;
        this.resumePage();
      }, delay),
    };
  }
  private initialRange(): OrderRange {
    const rows = this.cache.rows;
    const index = Math.max(
      0,
      rows.findIndex((row) => row.key === this.anchor),
    );
    const start = Math.max(0, index - 20);
    let bound =
      start === 0 && !this.cache.hasBefore ? null : (rows[start]?.key ?? null);
    const island = rows[index] && this.cache.coveredRange(rows[index].key);
    if (island?.start != null && (bound === null || bound < island.start))
      bound = island.start;
    return { start: bound, end: null };
  }
  private knownRange(): OrderRange {
    const rows = this.cache.rows;
    const index = Math.max(
      0,
      rows.findIndex((row) =>
        this.visibleKey
          ? orderKey(row) === this.visibleKey
          : row.key === this.anchor,
      ),
    );
    const island = this.cache.coveredRange(rows[index].key)!;
    const start = Math.max(0, index - 20),
      end = Math.min(rows.length, start + WATCH_SIZE);
    let left = start === 0 && !this.cache.hasBefore ? null : rows[start].key;
    let right = end === rows.length ? island.end : rows[end].key;
    if (island.start !== null && (left === null || left < island.start))
      left = island.start;
    if (island.end !== null && (right === null || right > island.end))
      right = island.end;
    return { start: left, end: right };
  }
  private watchKnown() {
    if (this.watch.kind !== "live" || !this.cache.rows.length) return;
    const range = this.knownRange();
    if (JSON.stringify(range) === JSON.stringify(this.watch.view.range)) return;
    this.source.watch(this.query(range, WATCH_SIZE));
  }
  private queueViewport() {
    clearTimeout(this.movement);
    if (!this.viewport || this.closed) return;
    this.movement = setTimeout(() => {
      this.movement = undefined;
      if (this.watch.kind !== "live" || this.pending || !this.viewport) return;
      const gap = this.cache.unreadWithin(this.viewport);
      if (gap && !this.pageError) {
        this.wantPage = true;
        this.resumePage();
        return;
      }
      const range = this.watch.view.range;
      if (
        (range.start !== null &&
          this.viewport.start !== null &&
          this.viewport.start < range.start) ||
        (range.end !== null &&
          this.viewport.end !== null &&
          this.viewport.end >= range.end)
      )
        this.watchKnown();
    }, 80);
  }
  private async load(range: OrderRange) {
    if (this.watch.kind !== "live" || this.pending) return;
    const stop = new AbortController();
    this.pending = stop;
    this.pageError = "";
    this.activity.rebase();
    this.emit();
    try {
      const page = await this.source.query(this.query(range), stop.signal);
      if (this.closed || stop.signal.aborted) return;
      this.failures = 0;
      this.wantPage = false;
      this.accept(page);
      if (!page.data.length && this.cache.unread) {
        this.wantPage = true;
        this.retryPage(80);
      }
    } catch (error) {
      if (this.closed || stop.signal.aborted) return;
      this.pageError = error instanceof Error ? error.message : String(error);
      const transient =
        !(error instanceof ApiFailure) ||
        ["unavailable", "timed_out", "capacity_exceeded"].includes(error.code);
      this.wantPage = transient;
      if (transient) this.retryPage(retryDelay(this.failures++), "backoff");
    } finally {
      if (this.pending === stop) {
        this.pending = undefined;
        this.emit();
        if (!this.closed) {
          if (stop.signal.reason === RESTART_PAGE) this.retryPage(80);
          this.queueViewport();
        }
      }
    }
  }
  private accept(data: OrderData, requested?: OrderRange) {
    this.cache.merge(data, requested);
    // A finite read must not overwrite a newer observation from the active watch.
    if (this.watch.kind === "live" && data !== this.watch.view)
      this.cache.merge(this.watch.view);
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
    this.emit({ orders, ...this.activity.update(orders) });
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
  private emit(patch: Partial<FeedState> = {}) {
    if (this.closed) return;
    const error =
      this.watch.kind === "failed" ? this.watch.error : this.pageError;
    const status = error
      ? "Unavailable"
      : this.watch.kind === "live"
        ? this.availability === "available"
          ? "Live"
          : this.availability === "delayed"
            ? "Updates delayed"
            : "Unavailable"
        : this.watch.kind === "reconnecting"
          ? "Reconnecting"
          : "Connecting";
    this.state = {
      ...this.state,
      ...patch,
      status,
      error,
      hasMore: this.ready && !!this.cache.unread,
      loadingMore:
        !!this.pending || (!this.ready && this.watch.kind !== "failed"),
    };
    this.changed(this.state);
  }
}
