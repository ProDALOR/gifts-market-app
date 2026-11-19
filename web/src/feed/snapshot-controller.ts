import { FrozenReads, type Snapshot } from "./range-source.ts";
import { ApiFailure, type OrderQuery } from "../market.ts";
import { LoadedOrders } from "./loaded-orders.ts";
import { orderKey } from "../orderbook/order-activity.ts";
import {
  emptyFeed,
  type Bookmark,
  type FeedController,
  type FeedState,
} from "./types.ts";
/** One immutable view for explicitly paused reads. Expiry rebuilds all visited rows before publishing. */
export class SnapshotController implements FeedController {
  private stop = new AbortController();
  private source = new FrozenReads();
  private state = emptyFeed();
  private pages: Snapshot[] = [];
  private cache: LoadedOrders;
  private anchor?: string;
  private working = false;
  private wanted: number;
  private through?: string;
  private readonly size: number;
  ready = false;
  constructor(
    private query: OrderQuery,
    private changed: (state: FeedState) => void,
    bookmark?: Bookmark,
  ) {
    this.size = query.limit ?? 100;
    this.cache = bookmark?.cache ?? new LoadedOrders();
    this.anchor = bookmark?.anchor;
    this.wanted = Math.max(1, this.cache.rows.length);
    this.through = this.cache.rows.at(-1)?.key;
  }
  get bookmark(): Bookmark {
    return {
      cache: this.cache,
      anchor: this.anchor,
    };
  }
  start() {
    void this.load();
  }
  close() {
    this.stop.abort();
    this.source.close();
  }
  setVisibleRange(start: number, _end: number, anchor = start) {
    this.anchor = this.cache.rows[anchor]?.key ?? this.anchor;
  }
  loadMore() {
    if (!this.working && !this.state.error && this.state.hasMore) {
      this.wanted = this.cache.rows.length + this.size;
      void this.load();
    }
  }
  private async load() {
    if (this.working || this.stop.signal.aborted) return;
    this.working = true;
    this.emit({ loadingMore: true, status: "Loading" });
    let restarted = false;
    try {
      let count = this.pages.reduce((n, p) => n + p.data.length, 0);
      while (true) {
        const start = this.pages.at(-1)?.range.end ?? null;
        // Cached islands can contain unread gaps: row count alone can stop
        // before the previously visited tail when a frozen view is rebuilt.
        if (
          this.pages.length &&
          (start === null ||
            (count >= this.wanted &&
              (this.through === undefined || start > this.through)))
        )
          break;
        try {
          const page = await this.source.query(
            { ...this.query, limit: this.size, range: { start, end: null } },
            this.stop.signal,
          );
          if (this.stop.signal.aborted) return;
          this.pages.push(page);
          count += page.data.length;
        } catch (error) {
          if (
            error instanceof ApiFailure &&
            error.code === "restart_read" &&
            !restarted
          ) {
            restarted = true;
            this.pages = [];
            count = 0;
            continue;
          }
          throw error;
        }
      }
      const seen = new Set<string>();
      const data = this.pages.flatMap((page) =>
        page.data.filter((row) => {
          const id = orderKey(row);
          if (seen.has(id)) return false;
          seen.add(id);
          return true;
        }),
      );
      const end = this.pages.at(-1)?.range.end ?? null;
      const cache = new LoadedOrders();
      cache.merge({ data, range: { start: null, end } });
      this.cache = cache;
      this.through = cache.rows.at(-1)?.key;
      const orders = cache.rows.map(({ key: _, ...row }) => row);
      this.ready = true;
      this.emit({
        orders,
        displayOrders: orders,
        hasMore: end !== null,
        status: "Paused",
        error: "",
      });
    } catch (error) {
      this.emit({
        error: error instanceof Error ? error.message : String(error),
        status: "Unavailable",
      });
    } finally {
      this.working = false;
      this.emit({ loadingMore: false });
    }
  }
  private emit(patch: Partial<FeedState>) {
    if (!this.stop.signal.aborted) {
      this.state = { ...this.state, ...patch };
      this.changed(this.state);
    }
  }
}
