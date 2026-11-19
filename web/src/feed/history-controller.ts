import { release } from "./range-source.ts";
import {
  request,
  ApiFailure,
  type HistoryQuery,
  type OrderQueryResult,
} from "../market.ts";
import { orderKey } from "../orderbook/order-activity.ts";
import {
  emptyFeed,
  type Bookmark,
  type FeedController,
  type FeedState,
} from "./types.ts";
/** Forward pagination belongs only to immutable execution-history snapshots. */
export class HistoryController implements FeedController {
  private stop = new AbortController();
  private state = emptyFeed();
  private pages: OrderQueryResult[] = [];
  private snapshot?: string;
  private working = false;
  private wanted: number;
  ready = false;
  constructor(
    private query: HistoryQuery,
    private changed: (state: FeedState) => void,
    bookmark?: Bookmark,
  ) {
    this.wanted = bookmark?.kind === "history" ? bookmark.depth : 1;
  }
  get bookmark(): Bookmark {
    return { kind: "history", depth: this.wanted };
  }
  start() {
    void this.load();
  }
  close() {
    this.stop.abort();
    if (this.snapshot) release(this.snapshot);
  }
  setVisibleRange() {}
  loadMore() {
    if (!this.working && !this.state.error && this.state.hasMore) {
      this.wanted++;
      void this.load();
    }
  }
  private async load() {
    if (this.working) return;
    this.working = true;
    this.emit({
      loadingMore: true,
      status: this.ready ? "Snapshot" : "Loading",
    });
    let restarted = false;
    try {
      while (this.pages.length < this.wanted) {
        const token = this.pages.at(-1)?.next || null;
        if (this.pages.length && !token) break;
        try {
          const page = await request<OrderQueryResult>(
            "/orders/query",
            {
              ...this.query,
              source: "history",
              navigation: token
                ? { type: "after", key: token }
                : { type: "head" },
              view: { kind: "snapshot", id: this.snapshot ?? null },
            },
            this.stop.signal,
          );
          if (this.stop.signal.aborted) {
            if (page.snapshot) release(page.snapshot);
            return;
          }
          if (!page.snapshot)
            throw new Error("Snapshot lease missing from history read");
          this.snapshot = page.snapshot;
          if (
            page.next &&
            (page.next === token ||
              this.pages.some((p) => p.next === page.next))
          )
            throw new Error("Market returned a repeated continuation");
          this.pages.push(page);
        } catch (error) {
          if (
            error instanceof ApiFailure &&
            error.code === "restart_read" &&
            !restarted
          ) {
            restarted = true;
            this.pages = [];
            if (this.snapshot) release(this.snapshot);
            this.snapshot = undefined;
            continue;
          }
          throw error;
        }
      }
      const seen = new Set<string>();
      const orders = this.pages.flatMap((page) =>
        page.data.filter((row) => {
          const key = orderKey(row);
          if (seen.has(key)) return false;
          seen.add(key);
          return true;
        }),
      );
      this.ready = true;
      this.emit({
        orders,
        displayOrders: orders,
        hasMore: !!this.pages.at(-1)?.next,
        status: "Snapshot",
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
